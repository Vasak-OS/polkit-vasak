//! polkit-agent-helper-dbus
//!
//! Trusted, setuid-root helper for the VasakOS PolicyKit agent.
//!
//! polkitd only accepts `AuthenticationAgentResponse3` from a uid-0 process, so
//! this helper is installed setuid root (exactly like the stock
//! `polkit-agent-helper-1`). Because it runs as root, it — and NOT the
//! unprivileged agent — is the component that must verify the user's password.
//!
//! Flow:
//!   1. Refuse to run unless the setuid bit is in effect; become fully root.
//!   2. Read the cookie and password from stdin (never argv, so they don't leak
//!      via `ps`/`/proc/<pid>/cmdline`).
//!   3. Authenticate, via PAM (`polkit-1` service), the *exact* identity polkit
//!      asked for (resolved from `--identity-uid`), not the calling user.
//!   4. Only on PAM success, send `AuthenticationAgentResponse3` binding the
//!      response to the requesting process (pid + pidfd + start-time).
//!
//! A local attacker running this helper directly still has to supply the
//! correct password for the requested identity (e.g. root's password for an
//! `auth_admin` action), and polkitd independently checks that the reported
//! identity is one it offered for the cookie. There is no free escalation.

use std::collections::HashMap;
use std::env;
use std::ffi::CStr;
use std::io::BufRead;
use std::os::fd::{FromRawFd, OwnedFd};
use zbus::Connection;
use zvariant::{Fd, Value};

fn get_arg(args: &[String], name: &str) -> String {
    let pos = args.iter().position(|a| a == name).unwrap_or_else(|| {
        eprintln!("Missing argument: {name}");
        std::process::exit(1);
    });
    args.get(pos + 1).cloned().unwrap_or_else(|| {
        eprintln!("Missing value for argument: {name}");
        std::process::exit(1);
    })
}

fn get_start_time(pid: u32) -> u64 {
    let path = format!("/proc/{pid}/stat");
    let data = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        eprintln!("Failed to read {path}: {e}");
        std::process::exit(1);
    });

    // The comm field may contain spaces/parens, so scan past the final ") ".
    let after_comm = data.rfind(") ").unwrap_or_else(|| {
        eprintln!("Cannot parse {path}: no ') ' found");
        std::process::exit(1);
    });
    let rest = &data[after_comm + 2..];
    let fields: Vec<&str> = rest.split_whitespace().collect();

    fields.get(19).and_then(|s| s.parse::<u64>().ok()).unwrap_or_else(|| {
        eprintln!("Cannot parse starttime from {path}");
        std::process::exit(1);
    })
}

/// Resolve a username from a uid via the passwd database.
///
/// `getpwuid` is not reentrant, but this process is single-threaded at this
/// point (the Tokio runtime is only created afterwards).
fn username_for_uid(uid: u32) -> Option<String> {
    unsafe {
        let pw = libc::getpwuid(uid as libc::uid_t);
        if pw.is_null() {
            return None;
        }
        CStr::from_ptr((*pw).pw_name).to_str().ok().map(|s| s.to_string())
    }
}

/// Authenticate `user` with `password` through the `polkit-1` PAM stack.
///
/// Runs as root (see setuid hardening in `main`), which is required to read
/// `/etc/shadow` when authenticating an identity other than the caller.
fn pam_authenticate(user: &str, password: &str) -> bool {
    let mut client = match pam::Client::with_password("polkit-1") {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[polkit-helper] PAM init failed: {e}");
            return false;
        }
    };
    client.conversation_mut().set_credentials(user, password);
    client.authenticate().is_ok()
}

fn main() {
    // --- setuid hardening -------------------------------------------------
    // polkitd only accepts a response from uid 0, so this binary is installed
    // setuid root. Refuse to run if that bit is not in effect, and become fully
    // root so PAM/shadow access behaves predictably.
    if unsafe { libc::geteuid() } != 0 {
        eprintln!("[polkit-helper] must run as root (setuid bit missing)");
        std::process::exit(1);
    }
    if unsafe { libc::setreuid(0, 0) } != 0 {
        eprintln!("[polkit-helper] setreuid(0,0) failed");
        std::process::exit(1);
    }
    // A setuid process must not trust a caller-supplied system bus address.
    env::remove_var("DBUS_SYSTEM_BUS_ADDRESS");

    let args: Vec<String> = env::args().collect();
    let identity_kind = get_arg(&args, "--identity-kind");
    let identity_uid: u32 = get_arg(&args, "--identity-uid").parse().unwrap_or_else(|_| {
        eprintln!("[polkit-helper] invalid --identity-uid");
        std::process::exit(1);
    });
    let subject_pid: u32 = get_arg(&args, "--subject-pid").parse().unwrap_or_else(|_| {
        eprintln!("[polkit-helper] invalid --subject-pid");
        std::process::exit(1);
    });

    // We authenticate a uid against the passwd DB, so only unix-user identities
    // are supported (which is what polkit offers for authentication anyway).
    if identity_kind != "unix-user" {
        eprintln!("[polkit-helper] unsupported identity kind: {identity_kind}");
        std::process::exit(1);
    }

    // --- secrets come from stdin, never argv ------------------------------
    // Line 1: cookie, line 2: password. Keeping them off argv avoids leaking
    // them through `ps` / `/proc/<pid>/cmdline`.
    let stdin = std::io::stdin();
    let mut lines = stdin.lock().lines();
    let cookie = lines.next().and_then(|r| r.ok()).unwrap_or_else(|| {
        eprintln!("[polkit-helper] missing cookie on stdin");
        std::process::exit(1);
    });
    let password = lines.next().and_then(|r| r.ok()).unwrap_or_else(|| {
        eprintln!("[polkit-helper] missing password on stdin");
        std::process::exit(1);
    });

    // --- authenticate the requested identity BEFORE telling polkitd -------
    let username = username_for_uid(identity_uid).unwrap_or_else(|| {
        eprintln!("[polkit-helper] no user for uid {identity_uid}");
        std::process::exit(1);
    });
    if !pam_authenticate(&username, &password) {
        eprintln!("[polkit-helper] PAM authentication failed for {username}");
        std::process::exit(1);
    }

    // pidfd + start-time bind the response to the exact requesting process,
    // closing the pid-reuse (TOCTOU) window.
    let pidfd = unsafe {
        libc::syscall(libc::SYS_pidfd_open, subject_pid as libc::pid_t, 0) as libc::c_int
    };
    if pidfd < 0 {
        eprintln!(
            "[polkit-helper] pidfd_open({subject_pid}) failed: {}",
            std::io::Error::last_os_error()
        );
        std::process::exit(1);
    }
    let start_time = get_start_time(subject_pid);

    let rt = tokio::runtime::Runtime::new().expect("tokio rt");
    rt.block_on(async {
        let conn = Connection::system().await.unwrap_or_else(|e| {
            eprintln!("[polkit-helper] system bus: {e}");
            std::process::exit(1);
        });

        let mut identity_details = HashMap::new();
        identity_details.insert("uid".to_string(), Value::U32(identity_uid));
        let identity: (&str, HashMap<String, Value>) = (identity_kind.as_str(), identity_details);

        let mut subject_details = HashMap::new();
        subject_details.insert("pid".to_string(), Value::U32(subject_pid));
        let owned_fd = unsafe { OwnedFd::from_raw_fd(pidfd) };
        subject_details.insert("pidfd".to_string(), Value::Fd(Fd::from(owned_fd)));
        subject_details.insert("start-time".to_string(), Value::U64(start_time));
        let subject: (&str, HashMap<String, Value>) = ("unix-process", subject_details);

        let result = conn
            .call_method(
                Some("org.freedesktop.PolicyKit1"),
                "/org/freedesktop/PolicyKit1/Authority",
                Some("org.freedesktop.PolicyKit1.Authority"),
                "AuthenticationAgentResponse3",
                &(cookie.as_str(), identity, subject),
            )
            .await;

        match result {
            Ok(_) => println!("SUCCESS"),
            Err(e) => {
                eprintln!("[polkit-helper] response error: {e}");
                std::process::exit(1);
            }
        }
    });
}
