mod desbloqueo;
mod llavero;
mod locales;
mod udisks;

use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::oneshot;
use zbus::interface;
use zvariant::{OwnedValue, Value};

struct AppState {
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<String>>>>,
}

// Genérico en el runtime sólo para que las pruebas usen el de mentira de
// Tauri (`tauri::test`): en producción siempre es el real.
struct PolkitAgent<R: tauri::Runtime> {
    app_handle: AppHandle<R>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<String>>>>,
    /// El turno del diálogo: la ventana muestra un pedido por vez. Sin esto, con
    /// los métodos en `&self`, un segundo pedido —un `pkexec` cualquiera— pisaba
    /// el texto del que estaba a la vista y se llevaba la contraseña que la
    /// persona escribía para lo que estaba haciendo.
    dialog: Arc<tokio::sync::Mutex<()>>,
}

/// Una identidad que polkit ofrece para autenticar: tipo y detalles (`uid`).
type Identity = (String, HashMap<String, OwnedValue>);

/// A quién se le pide la contraseña: a quien usa el sistema si polkit lo ofrece
/// —en `auth_admin` llegan todos los de `wheel`, y tomar el primero validaba
/// la contraseña contra la cuenta de otro y le sumaba intentos fallidos
/// (`polkit-vasak#36`)—, y si no, la primera identidad.
fn pick_identity(identities: Vec<Identity>, own_uid: u32) -> Option<Identity> {
    let own = identities.iter().position(|(kind, details)| {
        kind == "unix-user"
            && details
                .get("uid")
                .and_then(|v| u32::try_from(v).ok())
                .is_some_and(|uid| uid == own_uid)
    });
    identities.into_iter().nth(own.unwrap_or(0))
}

#[interface(name = "org.freedesktop.PolicyKit1.AuthenticationAgent")]
impl<R: tauri::Runtime> PolkitAgent<R> {
    // `&self` y no `&mut self`: zbus mantiene el candado de escritura de la
    // interfaz mientras corre un método `&mut`, y éste espera a la persona, así
    // que `cancel_authentication` y las demás pedidas quedaban trabadas detrás
    // (`polkit-vasak#37`). El estado ya va en `Arc<Mutex<…>>`.
    async fn begin_authentication(
        &self,
        action_id: &str,
        message: &str,
        _icon_name: &str,
        details: HashMap<String, String>,
        cookie: &str,
        identities: Vec<Identity>,
    ) {
        eprintln!("[vasak-polkit] begin_authentication action_id={action_id}");

        let cookie_owned = cookie.to_string();
        let message_owned = message.to_string();
        let action_id_owned = action_id.to_string();
        let subject_pid: u32 = details
            .get("polkit.subject-pid")
            .and_then(|p| p.parse().ok())
            .unwrap_or(0);
        // SAFETY: `getuid` no falla ni toca memoria.
        let identity = pick_identity(identities, unsafe { libc::getuid() });
        let identity_kind = identity
            .as_ref()
            .map(|(k, _)| k.clone())
            .unwrap_or_else(|| "unix-user".to_string());
        // Authenticate the *exact* identity polkit asked for. Never default to
        // an arbitrary uid (a previous `unwrap_or(1000)` let auth_admin actions
        // be validated against the wrong account).
        let identity_uid: Option<u32> = identity
            .as_ref()
            .and_then(|(_, d)| d.get("uid"))
            .and_then(|v| u32::try_from(v).ok());

        let (password_tx, password_rx) = oneshot::channel::<String>();
        let (done_tx, done_rx) = oneshot::channel::<bool>();

        self.pending
            .lock()
            .expect("lock pending")
            .insert(cookie_owned.clone(), password_tx);

        let cookie_task = cookie_owned.clone();
        let subject_pid_task = subject_pid;
        let identity_kind_task = identity_kind.clone();

        tokio::spawn(async move {
            let password = match password_rx.await {
                Ok(pwd) => pwd,
                Err(_) => {
                    let _ = done_tx.send(false);
                    return;
                }
            };

            // Without a concrete identity from polkit we cannot authenticate;
            // fail closed rather than guessing an account.
            let uid = match identity_uid {
                Some(uid) => uid,
                None => {
                    eprintln!("[vasak-polkit] no identity uid supplied by polkit");
                    let _ = done_tx.send(false);
                    return;
                }
            };

            // The setuid helper performs the PAM check (as root, for `uid`) and
            // only then answers polkitd. The agent never authenticates itself.
            let ok = call_authentication_helper(
                password,
                cookie_task,
                identity_kind_task,
                uid,
                subject_pid_task,
            )
            .await;

            let _ = done_tx.send(ok);
        });

        let _turn = self.dialog.lock().await;
        // Si polkitd la canceló mientras esperaba su turno, no hay qué mostrar.
        if !self
            .pending
            .lock()
            .expect("lock pending")
            .contains_key(&cookie_owned)
        {
            return;
        }

        // Show window first so the Vue transition is visible
        if let Some(window) = self.app_handle.get_webview_window("main") {
            let _ = window.show();
            let _ = window.set_focus();
        }
        let _ = self.app_handle.emit(
            "polkit-request",
            serde_json::json!({
                "message": message_owned,
                "cookie": cookie_owned,
            }),
        );

        // Block until auth flow completes — polkitd removes the session
        // from active_sessions immediately after begin_authentication returns.
        let success = done_rx.await.unwrap_or(false);

        let _ = self.app_handle.emit(
            "polkit-result",
            serde_json::json!({
                "success": success,
                "cookie": cookie_owned,
                "action_id": action_id_owned,
                "message": if success { "" } else { "Autenticación fallida" },
            }),
        );

        eprintln!("[vasak-polkit] auth result success={success}");

        if success {
            // Let the Vue leave transition play before hiding the window
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            if let Some(window) = self.app_handle.get_webview_window("main") {
                let _ = window.hide();
            }
        }
    }

    /// polkitd cancela por la **cookie** (`s`), no por una ruta de objeto: con
    /// `o` la llamada fallaba por la firma y cancelar no hacía nada. Soltar el
    /// emisor pendiente despierta a `begin_authentication`, que termina en falso.
    async fn cancel_authentication(&self, cookie: &str) {
        self.pending.lock().expect("lock pending").remove(cookie);

        let _ = self
            .app_handle
            .emit("polkit-cancel", serde_json::json!({ "cookie": cookie }));
    }
}

#[tauri::command]
async fn submit_password(
    state: State<'_, AppState>,
    password: String,
    cookie: String,
) -> Result<bool, String> {
    let tx = state
        .pending
        .lock()
        .map_err(|e| e.to_string())?
        .remove(&cookie)
        .ok_or_else(|| "No hay un pedido pendiente".to_string())?;

    tx.send(password)
        .map_err(|_| "Receiver dropped".to_string())?;
    Ok(true)
}

#[tauri::command]
async fn cancel_pending(state: State<'_, AppState>, cookie: String) -> Result<(), String> {
    state
        .pending
        .lock()
        .map_err(|e| e.to_string())?
        .remove(&cookie);
    Ok(())
}

/// Locate the setuid `polkit-agent-helper-dbus` binary.
///
/// Prefer a copy next to the running agent (dev builds), then the installed
/// path.
fn resolve_helper_path() -> Result<std::path::PathBuf, String> {
    let mut sibling = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    sibling.set_file_name("polkit-agent-helper-dbus");
    if sibling.exists() {
        return Ok(sibling);
    }
    let installed = std::path::PathBuf::from("/usr/bin/polkit-agent-helper-dbus");
    if installed.exists() {
        return Ok(installed);
    }
    Err("polkit-agent-helper-dbus not found".to_string())
}

/// Hand the request to the setuid helper, which authenticates `uid` via PAM and
/// answers polkitd. The cookie and password are written to the helper's stdin
/// (never argv) so they never appear in `ps`/`/proc/<pid>/cmdline`.
async fn call_authentication_helper(
    password: String,
    cookie: String,
    identity_kind: String,
    uid: u32,
    subject_pid: u32,
) -> bool {
    let result = tokio::task::spawn_blocking(move || -> Result<bool, String> {
        let helper_path = resolve_helper_path()?;

        let mut child = Command::new(&helper_path)
            .arg("--identity-kind")
            .arg(&identity_kind)
            .arg("--identity-uid")
            .arg(uid.to_string())
            .arg("--subject-pid")
            .arg(subject_pid.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("spawn helper: {e}"))?;

        {
            use std::io::Write;
            let stdin = child
                .stdin
                .as_mut()
                .ok_or_else(|| "helper stdin unavailable".to_string())?;
            // Line 1: cookie, then the password (which may itself contain
            // newlines — the helper takes everything after the first one).
            stdin
                .write_all(format!("{cookie}\n{password}\n").as_bytes())
                .map_err(|e| format!("write helper stdin: {e}"))?;
        }

        let output = child
            .wait_with_output()
            .map_err(|e| format!("wait helper: {e}"))?;

        if output.status.success() {
            Ok(true)
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            Err(format!("helper failed: {stderr}"))
        }
    })
    .await;

    match result {
        Ok(Ok(ok)) => ok,
        Ok(Err(e)) => {
            eprintln!("[vasak-polkit] helper error: {e}");
            false
        }
        Err(e) => {
            eprintln!("[vasak-polkit] helper join error: {e}");
            false
        }
    }
}

/// La sesión gráfica del usuario según logind (`User.Display`), la misma que
/// polkit le atribuye a un proceso que no está en ninguna.
async fn display_session(conn: &zbus::Connection) -> Option<String> {
    let reply = conn
        .call_method(
            Some("org.freedesktop.login1"),
            "/org/freedesktop/login1/user/self",
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &("org.freedesktop.login1.User", "Display"),
        )
        .await
        .ok()?;
    let value: zbus::zvariant::OwnedValue = reply.body().deserialize().ok()?;
    session_id_of(&value)
}

/// El id de un `(so)` de logind; vacío es «sin sesión gráfica».
fn session_id_of(value: &Value<'_>) -> Option<String> {
    let value = match value {
        Value::Value(inner) => inner.as_ref(),
        other => other,
    };
    let Value::Structure(fields) = value else {
        return None;
    };
    match fields.fields().first() {
        Some(Value::Str(id)) if !id.is_empty() => Some(id.to_string()),
        _ => None,
    }
}

/// Las sesiones a probar, en orden y sin repetir; nunca ids inventados.
fn session_candidates(env: Option<String>, display: Option<String>) -> Vec<String> {
    let mut ids: Vec<String> = env.into_iter().chain(display).collect();
    ids.dedup();
    ids
}

async fn register_polkit_agent(
    app_handle: AppHandle,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<String>>>>,
    desbloqueos: desbloqueo::DialogosPendientes,
) {
    let conn = zbus::Connection::system()
        .await
        .expect("Failed to connect to system bus");

    let agent = PolkitAgent {
        app_handle: app_handle.clone(),
        pending: pending.clone(),
        dialog: Arc::default(),
    };

    conn.object_server()
        .at("/org/freedesktop/PolicyKit1/AuthenticationAgent", agent)
        .await
        .expect("Failed to register agent object on D-Bus");

    eprintln!(
        "[vasak-polkit] Bus unique name: {}",
        conn.unique_name().map(|n| n.as_str()).unwrap_or("?")
    );

    // Corriendo como servicio de usuario no hay `XDG_SESSION_ID` ni el proceso
    // está en ninguna sesión, y polkit sólo acepta la sesión en la que dice que
    // está quien llama: para un servicio, la gráfica del usuario. Antes se
    // probaban ids inventados (2, 3, 1), y si ninguno era el de esta sesión el
    // agente no se registraba: nadie pedía contraseña y montar un disco interno
    // fallaba con «no autorizado».
    let known_sessions = session_candidates(
        std::env::var("XDG_SESSION_ID").ok(),
        display_session(&conn).await,
    );

    let mut registered = false;
    for sid in known_sessions {
        let subject: (&str, HashMap<String, Value<'_>>) = (
            "unix-session",
            HashMap::from([("session-id".to_string(), Value::Str(sid.clone().into()))]),
        );

        let result = conn
            .call_method(
                Some("org.freedesktop.PolicyKit1"),
                "/org/freedesktop/PolicyKit1/Authority",
                Some("org.freedesktop.PolicyKit1.Authority"),
                "RegisterAuthenticationAgent",
                &(
                    &subject,
                    "en_US.UTF-8",
                    "/org/freedesktop/PolicyKit1/AuthenticationAgent",
                ),
            )
            .await;

        match result {
            Ok(_) => {
                eprintln!("[vasak-polkit] Registered successfully");
                registered = true;
                break;
            }
            Err(e) => {
                eprintln!("[vasak-polkit] A session candidate failed: {e}");
            }
        }
    }

    if !registered {
        eprintln!("[vasak-polkit] Failed to register with any session");
    }

    // El desbloqueo de discos cifrados va en el bus de sesión, y es
    // independiente de todo lo de arriba: que no se pueda publicar deja al
    // escritorio sin abrir discos, no sin autenticar. Ver `desbloqueo`.
    if let Err(error) = desbloqueo::publicar(app_handle, desbloqueos).await {
        eprintln!("[vasak-polkit] sin desbloqueo de discos: {error}");
    }

    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(3600)).await;
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let pending = Arc::new(Mutex::new(HashMap::new()));

    let desbloqueos = desbloqueo::DialogosPendientes::default();

    tauri::Builder::default()
        .manage(AppState {
            pending: pending.clone(),
        })
        .manage(desbloqueos.clone())
        .plugin(tauri_plugin_i18n_vsk::init_with_path(
            Some(locales::idioma_del_sistema()),
            locales::directorio(),
        ))
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_config_manager::init())
        .plugin(tauri_plugin_vicons::init())
        .setup(move |app| {
            let app_handle = app.handle().clone();
            let p = pending.clone();
            let d = desbloqueos.clone();

            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new()
                    .expect("Failed to create Tokio runtime for D-Bus agent");
                rt.block_on(async {
                    register_polkit_agent(app_handle, p, d).await;
                });
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            submit_password,
            cancel_pending,
            desbloqueo::enviar_frase,
            desbloqueo::cancelar_desbloqueo
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod pruebas_de_sesion {
    use super::*;
    use zvariant::{ObjectPath, StructureBuilder};

    fn display(id: &str) -> Value<'static> {
        Value::Structure(
            StructureBuilder::new()
                .add_field(id.to_string())
                .add_field(ObjectPath::try_from("/org/freedesktop/login1/session/_34").unwrap())
                .build(),
        )
    }

    #[test]
    fn el_id_sale_de_la_estructura_de_logind() {
        assert_eq!(session_id_of(&display("4")), Some("4".to_string()));
    }

    #[test]
    fn el_id_sale_aunque_la_estructura_venga_envuelta() {
        let envuelto = Value::Value(Box::new(display("4")));
        assert_eq!(session_id_of(&envuelto), Some("4".to_string()));
    }

    #[test]
    fn un_id_vacio_es_sin_sesion_grafica() {
        assert_eq!(session_id_of(&display("")), None);
    }

    #[test]
    fn lo_que_no_es_una_estructura_no_da_sesion() {
        assert_eq!(session_id_of(&Value::Str("4".into())), None);
    }

    #[test]
    fn los_candidatos_son_el_entorno_y_la_grafica_sin_ids_inventados() {
        assert_eq!(
            session_candidates(None, Some("4".into())),
            vec!["4".to_string()]
        );
        assert_eq!(
            session_candidates(Some("7".into()), Some("4".into())),
            vec!["7".to_string(), "4".to_string()]
        );
        assert!(session_candidates(None, None).is_empty());
    }

    #[test]
    fn un_candidato_repetido_se_prueba_una_vez() {
        assert_eq!(
            session_candidates(Some("4".into()), Some("4".into())),
            vec!["4".to_string()]
        );
    }

    /// Un logind falso al otro lado de un par de sockets: sin bus ni daemon.
    mod logind_falso {
        use super::*;
        use tokio::net::UnixStream;
        use zvariant::OwnedObjectPath;

        struct Usuario(String);

        #[zbus::interface(name = "org.freedesktop.login1.User")]
        impl Usuario {
            #[zbus(property)]
            fn display(&self) -> (String, OwnedObjectPath) {
                (
                    self.0.clone(),
                    OwnedObjectPath::try_from("/org/freedesktop/login1/session/_34").unwrap(),
                )
            }
        }

        /// El cliente de `display_session`, con el servidor sirviendo `usuario`.
        async fn cliente(usuario: Option<&str>) -> (zbus::Connection, zbus::Connection) {
            let (a, b) = UnixStream::pair().unwrap();
            let servidor = zbus::connection::Builder::unix_stream(a)
                .server(zbus::Guid::generate())
                .unwrap()
                .p2p();
            // Sin ningún objeto servido el servidor no contesta ni con error;
            // «sin el objeto» es servir otro, no ninguno.
            let (ruta, id) = match usuario {
                Some(id) => ("/org/freedesktop/login1/user/self", id),
                None => ("/org/freedesktop/login1/user/otro", ""),
            };
            let servidor = servidor.serve_at(ruta, Usuario(id.to_string())).unwrap();
            let cliente = zbus::connection::Builder::unix_stream(b).p2p();
            let (servidor, cliente) = tokio::join!(servidor.build(), cliente.build());
            (servidor.unwrap(), cliente.unwrap())
        }

        #[tokio::test]
        async fn la_sesion_grafica_sale_de_logind() {
            let (_servidor, conexion) = cliente(Some("4")).await;
            assert_eq!(display_session(&conexion).await, Some("4".to_string()));
        }

        #[tokio::test]
        async fn un_usuario_sin_sesion_grafica_no_da_sesion() {
            let (_servidor, conexion) = cliente(Some("")).await;
            assert_eq!(display_session(&conexion).await, None);
        }

        #[tokio::test]
        async fn sin_el_objeto_de_logind_no_da_sesion() {
            let (_servidor, conexion) = cliente(None).await;
            assert_eq!(display_session(&conexion).await, None);
        }
    }

    fn identidad(tipo: &str, uid: u32) -> Identity {
        let mut detalles = HashMap::new();
        detalles.insert("uid".to_string(), Value::from(uid).try_into().unwrap());
        (tipo.to_string(), detalles)
    }

    fn uid_de(identidad: &Identity) -> u32 {
        u32::try_from(identidad.1.get("uid").unwrap()).unwrap()
    }

    #[test]
    fn si_polkit_ofrece_a_quien_usa_el_sistema_se_le_pide_a_el_aunque_no_sea_el_primero() {
        let ofrecidas = vec![
            identidad("unix-user", 1001),
            identidad("unix-user", 1000),
            identidad("unix-user", 1002),
        ];
        let elegida = pick_identity(ofrecidas, 1000).unwrap();
        assert_eq!(uid_de(&elegida), 1000);
    }

    #[test]
    fn si_quien_usa_el_sistema_no_esta_ofrecido_se_toma_la_primera() {
        let ofrecidas = vec![identidad("unix-user", 1001), identidad("unix-user", 1002)];
        let elegida = pick_identity(ofrecidas, 1000).unwrap();
        assert_eq!(uid_de(&elegida), 1001);
    }

    #[test]
    fn sin_identidades_no_hay_a_quien_pedirle_la_contrasena() {
        assert!(pick_identity(Vec::new(), 1000).is_none());
    }

    #[test]
    fn una_identidad_que_no_es_unix_user_con_el_mismo_uid_no_cuenta_como_propia() {
        let ofrecidas = vec![
            identidad("unix-group", 1000),
            identidad("unix-user", 1001),
            identidad("unix-user", 1000),
        ];
        let elegida = pick_identity(ofrecidas, 1000).unwrap();
        assert_eq!((elegida.0.as_str(), uid_de(&elegida)), ("unix-user", 1000));
    }

    /// El agente servido por un par de sockets, sin bus ni ventana: el runtime
    /// de mentira de Tauri alcanza porque `begin_authentication` sólo emite
    /// eventos y busca una ventana que acá no existe.
    mod agente_por_dbus {
        use super::*;
        use std::time::Duration;
        use tauri::test::{mock_app, MockRuntime};
        use tokio::net::UnixStream;

        const RUTA: &str = "/org/freedesktop/PolicyKit1/AuthenticationAgent";
        const INTERFAZ: &str = "org.freedesktop.PolicyKit1.AuthenticationAgent";
        const PLAZO: Duration = Duration::from_secs(5);

        type Pendientes = Arc<Mutex<HashMap<String, oneshot::Sender<String>>>>;

        async fn montar() -> (
            zbus::Connection,
            zbus::Connection,
            Pendientes,
            tauri::App<MockRuntime>,
        ) {
            let app = mock_app();
            let pendientes: Pendientes = Arc::default();
            let agente = PolkitAgent {
                app_handle: app.handle().clone(),
                pending: pendientes.clone(),
                dialog: Arc::default(),
            };
            let (a, b) = UnixStream::pair().unwrap();
            let servidor = zbus::connection::Builder::unix_stream(a)
                .server(zbus::Guid::generate())
                .unwrap()
                .p2p()
                .serve_at(RUTA, agente)
                .unwrap();
            let cliente = zbus::connection::Builder::unix_stream(b).p2p();
            let (servidor, cliente) = tokio::join!(servidor.build(), cliente.build());
            (servidor.unwrap(), cliente.unwrap(), pendientes, app)
        }

        /// `BeginAuthentication` sin identidades: queda esperando la contraseña.
        fn empezar(
            cliente: &zbus::Connection,
            cookie: &'static str,
        ) -> tokio::task::JoinHandle<zbus::Result<()>> {
            let cliente = cliente.clone();
            tokio::spawn(async move {
                let cuerpo = (
                    "org.ejemplo.accion",
                    "mensaje",
                    "",
                    HashMap::<String, String>::new(),
                    cookie,
                    Vec::<Identity>::new(),
                );
                cliente
                    .call_method(
                        None::<&str>,
                        RUTA,
                        Some(INTERFAZ),
                        "BeginAuthentication",
                        &cuerpo,
                    )
                    .await
                    .map(|_| ())
            })
        }

        async fn cancelar(cliente: &zbus::Connection, cookie: &str) -> zbus::Result<()> {
            tokio::time::timeout(
                PLAZO,
                cliente.call_method(
                    None::<&str>,
                    RUTA,
                    Some(INTERFAZ),
                    "CancelAuthentication",
                    &(cookie,),
                ),
            )
            .await
            .expect("CancelAuthentication quedó trabada")
            .map(|_| ())
        }

        async fn esperar_pendiente(pendientes: &Pendientes, cookie: &str) {
            tokio::time::timeout(PLAZO, async {
                while !pendientes.lock().unwrap().contains_key(cookie) {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap_or_else(|_| panic!("BeginAuthentication de {cookie} no llegó a registrarse"));
        }

        #[tokio::test]
        async fn polkitd_cancela_con_la_cookie_y_la_llamada_se_acepta() {
            let (_servidor, cliente, _pendientes, _app) = montar().await;
            cancelar(&cliente, "cookie-que-no-existe").await.unwrap();
        }

        #[tokio::test]
        async fn cancelar_termina_la_autenticacion_pendiente_con_esa_cookie() {
            let (_servidor, cliente, pendientes, _app) = montar().await;
            let inicio = empezar(&cliente, "c1");
            esperar_pendiente(&pendientes, "c1").await;

            cancelar(&cliente, "c1").await.unwrap();

            tokio::time::timeout(PLAZO, inicio)
                .await
                .expect("la autenticación cancelada no terminó")
                .unwrap()
                .unwrap();
            assert!(pendientes.lock().unwrap().is_empty());
        }

        /// Las cookies de los `polkit-request` emitidos, en orden.
        fn pedidos_emitidos(app: &tauri::App<MockRuntime>) -> Arc<Mutex<Vec<String>>> {
            use tauri::Listener;
            let vistos: Arc<Mutex<Vec<String>>> = Arc::default();
            let destino = vistos.clone();
            app.listen("polkit-request", move |evento| {
                let carga: serde_json::Value = serde_json::from_str(evento.payload()).unwrap();
                destino
                    .lock()
                    .unwrap()
                    .push(carga["cookie"].as_str().unwrap().to_string());
            });
            vistos
        }

        async fn esperar_pedidos(vistos: &Arc<Mutex<Vec<String>>>, cuantos: usize) {
            tokio::time::timeout(PLAZO, async {
                while vistos.lock().unwrap().len() < cuantos {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap_or_else(|_| panic!("no llegaron {cuantos} polkit-request"));
        }

        #[tokio::test]
        async fn con_dos_pedidos_a_la_vez_el_segundo_espera_su_turno_para_mostrarse() {
            let (_servidor, cliente, pendientes, app) = montar().await;
            let vistos = pedidos_emitidos(&app);
            let primero = empezar(&cliente, "c1");
            esperar_pedidos(&vistos, 1).await;
            let segundo = empezar(&cliente, "c2");
            esperar_pendiente(&pendientes, "c2").await;
            for _ in 0..100 {
                tokio::task::yield_now().await;
            }
            assert_eq!(
                *vistos.lock().unwrap(),
                ["c1"],
                "el segundo pisaba al primero en el único diálogo"
            );

            cancelar(&cliente, "c1").await.unwrap();
            tokio::time::timeout(PLAZO, primero)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            esperar_pedidos(&vistos, 2).await;
            assert_eq!(*vistos.lock().unwrap(), ["c1", "c2"]);

            cancelar(&cliente, "c2").await.unwrap();
            tokio::time::timeout(PLAZO, segundo)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
        }

        #[tokio::test]
        async fn un_pedido_cancelado_mientras_espera_su_turno_no_se_muestra_y_termina() {
            let (_servidor, cliente, pendientes, app) = montar().await;
            let vistos = pedidos_emitidos(&app);
            let primero = empezar(&cliente, "c1");
            esperar_pedidos(&vistos, 1).await;
            let segundo = empezar(&cliente, "c2");
            esperar_pendiente(&pendientes, "c2").await;

            cancelar(&cliente, "c2").await.unwrap();
            cancelar(&cliente, "c1").await.unwrap();

            tokio::time::timeout(PLAZO, primero)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            tokio::time::timeout(PLAZO, segundo)
                .await
                .expect("el pedido cancelado en la cola no terminó")
                .unwrap()
                .unwrap();
            assert_eq!(*vistos.lock().unwrap(), ["c1"], "c2 no debía mostrarse");
        }

        #[tokio::test]
        async fn esperar_a_la_persona_no_traba_los_otros_pedidos() {
            let (_servidor, cliente, pendientes, _app) = montar().await;
            let primero = empezar(&cliente, "c1");
            esperar_pendiente(&pendientes, "c1").await;

            // Con `&mut self` el segundo pedido espera al primero y nunca se registra.
            let segundo = empezar(&cliente, "c2");
            esperar_pendiente(&pendientes, "c2").await;

            cancelar(&cliente, "c1").await.unwrap();
            tokio::time::timeout(PLAZO, primero)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            assert!(
                pendientes.lock().unwrap().contains_key("c2"),
                "cancelar una cookie no debe tocar las otras"
            );

            cancelar(&cliente, "c2").await.unwrap();
            tokio::time::timeout(PLAZO, segundo)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
        }
    }
}
