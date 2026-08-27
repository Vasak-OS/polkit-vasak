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
use std::io::Read;
use std::os::fd::{FromRawFd, OwnedFd};
use zbus::Connection;
use zeroize::Zeroizing;
use zvariant::{Fd, Value};

/// Hasta cuánto se lee de la entrada.
///
/// Esto corre como root y cualquiera del sistema puede ejecutarlo: sin techo,
/// alimentarle gigabytes por la entrada es agotar memoria con privilegios. Una
/// cookie y una contraseña no llegan ni a un kilobyte.
const LIMITE_ENTRADA: u64 = 64 * 1024;

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

/// Índice de `starttime` una vez saltado el `comm`.
///
/// En `/proc/<pid>/stat` es el campo 22 contando desde uno. Después del `) ` el
/// primero es el 3 —el estado—, así que 22 - 3 = 19. Equivocarse acá manda a
/// polkitd un tiempo de arranque que no es el del proceso, y la respuesta se
/// rechaza: el diálogo pide la contraseña, la acepta, y la acción no pasa.
const INDICE_STARTTIME: usize = 19;

/// El tiempo de arranque, a partir del contenido de `/proc/<pid>/stat`.
///
/// Separada de la lectura para poder probarla: el `comm` de un proceso puede
/// contener espacios y paréntesis, así que partir por espacios sin más deja todos
/// los campos corridos.
fn parse_start_time(data: &str) -> Option<u64> {
    // Se busca el **último** `") "`: un proceso puede llamarse `algo) raro`, y con
    // `find` en lugar de `rfind` el corte cae dentro del nombre.
    let after_comm = data.rfind(") ")?;
    data[after_comm + 2..]
        .split_whitespace()
        .nth(INDICE_STARTTIME)
        .and_then(|s| s.parse::<u64>().ok())
}

fn get_start_time(pid: u32) -> u64 {
    let path = format!("/proc/{pid}/stat");
    let data = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        eprintln!("Failed to read {path}: {e}");
        std::process::exit(1);
    });

    parse_start_time(&data).unwrap_or_else(|| {
        eprintln!("Cannot parse starttime from {path}");
        std::process::exit(1);
    })
}

/// Separa la cookie de la contraseña en lo que llegó por la entrada.
///
/// El formato es `cookie\ncontraseña\n`. La contraseña es **todo** lo que viene
/// después del primer salto, no la segunda línea: una contraseña con un salto
/// adentro se truncaba en silencio y la autenticación fallaba siempre, sin que
/// nada dijera por qué. La cookie la pone polkitd y no lleva saltos.
fn parse_stdin(datos: &str) -> Option<(String, Zeroizing<String>)> {
    let (cookie, resto) = datos.split_once('\n')?;
    if cookie.is_empty() {
        return None;
    }
    // Se quita **un** salto final, el que agrega el emisor. Los demás son parte de
    // la contraseña.
    let password = resto.strip_suffix('\n').unwrap_or(resto);
    Some((cookie.to_string(), Zeroizing::new(password.to_string())))
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
    let mut entrada = Zeroizing::new(String::new());
    if let Err(e) = std::io::stdin()
        .lock()
        .take(LIMITE_ENTRADA)
        .read_to_string(&mut entrada)
    {
        eprintln!("[polkit-helper] cannot read stdin: {e}");
        std::process::exit(1);
    }

    let (cookie, password) = parse_stdin(&entrada).unwrap_or_else(|| {
        eprintln!("[polkit-helper] malformed stdin (expected cookie and password)");
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Una línea real de `/proc/1/stat` de esta máquina.
    ///
    /// Real y no inventada: el formato tiene cincuenta y dos campos y una
    /// inventada puede coincidir con un índice equivocado por casualidad.
    const STAT_DE_SYSTEMD: &str = "1 (systemd) S 0 1 1 0 -1 4194560 61612 1335791 21509 2562 266 269 6941 3804 20 0 1 0 10 22003712 1171 18446744073709551615 1 1 0 0 0 0 671173123 4096 1260 0 0 0 17 6 0 0 0 0 0 0 0 0 0 0 0 0 0\n";

    #[test]
    fn el_tiempo_de_arranque_es_el_campo_veintidos() {
        // Si el índice está mal, se le manda a polkitd un tiempo que no es el del
        // proceso y la respuesta se rechaza: el diálogo pide la contraseña, la
        // acepta, y la acción no pasa. Falla de la forma más confusa posible.
        assert_eq!(parse_start_time(STAT_DE_SYSTEMD), Some(10));
    }

    #[test]
    fn un_nombre_de_proceso_con_espacios_no_corre_los_campos() {
        // El `comm` es lo que el proceso puso en su nombre, y puede tener espacios.
        // Partiendo por espacios sin saltarlo, todos los campos quedan corridos.
        let linea = STAT_DE_SYSTEMD.replace("(systemd)", "(un nombre con espacios)");
        assert_eq!(parse_start_time(&linea), Some(10));
    }

    #[test]
    fn un_nombre_con_parentesis_y_espacio_adentro_tampoco() {
        // Un proceso puede llamarse `algo) raro`. Buscando el **primer** `") "` en
        // lugar del último, el corte cae dentro del nombre.
        let linea = STAT_DE_SYSTEMD.replace("(systemd)", "(algo) raro)");
        assert_eq!(parse_start_time(&linea), Some(10));
    }

    #[test]
    fn una_linea_sin_sentido_no_devuelve_un_numero_inventado() {
        // Devolver un tiempo cualquiera sería peor que no devolver nada: ataría la
        // respuesta a un proceso que no es el que preguntó.
        assert_eq!(parse_start_time(""), None);
        assert_eq!(parse_start_time("sin parentesis"), None);
        assert_eq!(parse_start_time("1 (a) S 0 1"), None, "faltan campos");
        assert_eq!(parse_start_time("1 (a) S 0 1 1 0 -1 0 0 0 0 0 0 0 0 0 0 0 0 0 no-numero 0"), None);
    }

    #[test]
    fn la_cookie_y_la_contraseña_se_separan_en_el_primer_salto() {
        let (cookie, password) = parse_stdin("la-cookie\nla-contraseña\n").expect("parsea");
        assert_eq!(cookie, "la-cookie");
        assert_eq!(*password, "la-contraseña");
    }

    #[test]
    fn una_contraseña_con_saltos_adentro_no_se_trunca() {
        // Antes se leía sólo la segunda línea, así que una contraseña con un salto
        // se cortaba en silencio: la autenticación fallaba **siempre** y nada decía
        // por qué. La cookie la pone polkitd y no lleva saltos.
        let (cookie, password) = parse_stdin("c\nuna\ncon\nsaltos\n").expect("parsea");
        assert_eq!(cookie, "c");
        assert_eq!(*password, "una\ncon\nsaltos");
    }

    #[test]
    fn solo_se_quita_un_salto_final() {
        // El que agrega el emisor. Los demás son parte de la contraseña, y quitar
        // más haría fallar una contraseña que termina en un salto.
        let (_, password) = parse_stdin("c\nclave\n\n").expect("parsea");
        assert_eq!(*password, "clave\n");
    }

    #[test]
    fn una_contraseña_vacia_se_pasa_a_pam_y_no_se_acepta_sola() {
        // Que esté vacía no lo decide este helper: lo decide PAM, que con
        // `nullok` fuera es un rechazo. Lo que importa es que no se confunda con
        // una entrada mal formada.
        let (cookie, password) = parse_stdin("c\n\n").expect("parsea");
        assert_eq!(cookie, "c");
        assert_eq!(*password, "");
    }

    #[test]
    fn una_entrada_sin_salto_no_pasa() {
        // Sin salto no hay contraseña, sólo una cookie: seguir con una contraseña
        // vacía sería intentar autenticar con nada.
        assert_eq!(parse_stdin("solo-una-cookie").map(|(c, _)| c), None);
        assert_eq!(parse_stdin("").map(|(c, _)| c), None);
    }

    #[test]
    fn una_cookie_vacia_no_pasa() {
        // La cookie es lo que ata la respuesta a la pregunta de polkitd. Vacía, la
        // respuesta no corresponde a nada.
        assert_eq!(parse_stdin("\nsolo-contraseña\n").map(|(c, _)| c), None);
    }

    #[test]
    fn el_techo_de_entrada_alcanza_de_sobra_y_acota() {
        // Esto corre como root y cualquiera puede ejecutarlo: sin techo,
        // alimentarle gigabytes por la entrada agota memoria con privilegios.
        // 64 KiB: una cookie y una contraseña no llegan ni a un kilobyte, así que
        // sobra de largo, y acota lo que un local puede hacerle tragar.
        assert_eq!(LIMITE_ENTRADA, 64 * 1024);
    }

    #[test]
    fn el_argumento_repetido_toma_el_primero() {
        // Documenta el comportamiento en lugar de dejarlo al azar: quien ejecute
        // esto directamente igual tiene que saber la contraseña de la identidad
        // que pida, y polkitd comprueba por su lado que sea una que ofreció.
        let args: Vec<String> = ["--identity-uid", "1000", "--identity-uid", "0"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(get_arg(&args, "--identity-uid"), "1000");
    }
}
