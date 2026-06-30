use std::collections::HashMap;
use std::env;
use std::os::fd::{FromRawFd, OwnedFd};
use zbus::Connection;
use zvariant::{Fd, Value};

fn get_arg(args: &[String], name: &str) -> String {
    let pos = args.iter().position(|a| a == name).unwrap_or_else(|| {
        eprintln!("Missing argument: {name}");
        std::process::exit(1);
    });
    args.get(pos + 1)
        .cloned()
        .unwrap_or_else(|| {
            eprintln!("Missing value for argument: {name}");
            std::process::exit(1);
        })
}

fn get_start_time(pid: u32) -> u64 {
    let path = format!("/proc/{pid}/stat");
    let data = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| {
            eprintln!("Failed to read {path}: {e}");
            std::process::exit(1);
        });

    let after_comm = data.rfind(") ").unwrap_or_else(|| {
        eprintln!("Cannot parse {path}: no ') ' found");
        std::process::exit(1);
    });
    let rest = &data[after_comm + 2..];
    let fields: Vec<&str> = rest.split_whitespace().collect();

    fields.get(19)
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or_else(|| {
            eprintln!("Cannot parse starttime from {path}");
            std::process::exit(1);
        })
}

fn main() {
    let args: Vec<String> = env::args().collect();

    let cookie = get_arg(&args, "--cookie");
    let identity_kind = get_arg(&args, "--identity-kind");
    let identity_uid: u32 = get_arg(&args, "--identity-uid")
        .parse()
        .expect("valid uid");
    let subject_pid: u32 = get_arg(&args, "--subject-pid")
        .parse()
        .expect("valid pid");

    let pidfd = unsafe {
        libc::syscall(libc::SYS_pidfd_open, subject_pid as libc::pid_t, 0) as libc::c_int
    };
    if pidfd < 0 {
        eprintln!(
            "pidfd_open({subject_pid}) failed: {}",
            std::io::Error::last_os_error()
        );
        std::process::exit(1);
    }

    let start_time = get_start_time(subject_pid);

    let rt = tokio::runtime::Runtime::new().expect("tokio rt");
    rt.block_on(async {
        let conn = Connection::system().await.unwrap_or_else(|e| {
            eprintln!("Failed to connect to system bus: {e}");
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
                eprintln!("[pidfd-helper] ERROR: {e}");
                std::process::exit(1);
            }
        }
    });
}
