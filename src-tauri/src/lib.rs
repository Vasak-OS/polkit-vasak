use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::oneshot;
use tracing_subscriber;
use zbus::interface;
use zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

// ── Shared state ──────────────────────────────────────────────────

struct AppState {
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<String>>>>,
}

static SESSION_COUNTER: AtomicU64 = AtomicU64::new(1);

// ── D-Bus Polkit Agent ────────────────────────────────────────────

struct PolkitAgent {
    app_handle: AppHandle,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<String>>>>,
    session_map: Arc<Mutex<HashMap<String, String>>>,
    conn: zbus::Connection,
}

#[interface(name = "org.freedesktop.PolicyKit1.AuthenticationAgent")]
impl PolkitAgent {
    async fn begin_authentication(
        &mut self,
        _action_id: &str,
        message: &str,
        _icon_name: &str,
        _details: HashMap<String, String>,
        cookie: &str,
        _identities: Vec<(String, HashMap<String, OwnedValue>)>,
    ) -> OwnedObjectPath {
        eprintln!("[vasak-polkit] >>> begin_authentication CALLED");
        eprintln!("[vasak-polkit] >>> action_id={_action_id} cookie={cookie}");
        eprintln!("[vasak-polkit] >>> _identities count: {}", _identities.len());
        for (i, (kind, _det)) in _identities.iter().enumerate() {
            eprintln!("[vasak-polkit] >>> identity[{i}]: kind={kind}");
        }

        let cookie_owned = cookie.to_string();
        let message_owned = message.to_string();
        let action_id = _action_id.to_string();
        let session_num = SESSION_COUNTER.fetch_add(1, Ordering::Relaxed);
        let session_path_str =
            format!("/org/freedesktop/PolicyKit1/AuthenticationAgent/Session/{session_num}");

        let session_path: OwnedObjectPath = session_path_str
            .try_into()
            .expect("Static session path should always be valid");

        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .expect("lock pending")
            .insert(cookie_owned.clone(), tx);

        self.session_map
            .lock()
            .expect("lock session_map")
            .insert(session_path.to_string(), cookie_owned.clone());

        let app_handle = self.app_handle.clone();
        let conn = self.conn.clone();
        let cookie_task = cookie_owned.clone();

        tokio::spawn(async move {
            match rx.await {
                Ok(password) => {
                    let ok = authenticate_pam(&password).await;
                    eprintln!(
                        "[vasak-polkit] PAM result for cookie={cookie_task}: {ok}"
                    );

                    if ok {
                        let uid = unsafe { libc::getuid() };
                        let identity: (&str, HashMap<String, Value<'_>>) = (
                            "unix-user",
                            HashMap::from([(
                                "uid".to_string(),
                                Value::U32(uid),
                            )]),
                        );
                        let sid = std::env::var("XDG_SESSION_ID")
                            .unwrap_or_else(|_| "1".to_string());
                        let subject: (&str, HashMap<String, Value<'_>>) = (
                            "unix-session",
                            HashMap::from([(
                                "session-id".to_string(),
                                Value::Str(sid.into()),
                            )]),
                        );

                        let resp = conn
                            .call_method(
                                Some("org.freedesktop.PolicyKit1"),
                                "/org/freedesktop/PolicyKit1/Authority",
                                Some("org.freedesktop.PolicyKit1.Authority"),
                                "AuthenticationAgentResponse3",
                                &(&cookie_task, &identity, &subject),
                            )
                            .await;

                        if let Err(e) = resp {
                            eprintln!(
                                "[vasak-polkit] AuthenticationAgentResponse3 failed: {e}"
                            );
                        } else {
                            eprintln!(
                                "[vasak-polkit] AuthenticationAgentResponse3 OK"
                            );
                        }
                    }

                    let _ = app_handle.emit(
                        "polkit-result",
                        serde_json::json!({
                            "success": ok,
                            "cookie": cookie_task,
                            "action_id": action_id,
                        }),
                    );
                }
                Err(_) => {
                    eprintln!("[vasak-polkit] Auth cancelled (oneshot dropped)");
                }
            }
        });

        let _ = self.app_handle.emit(
            "polkit-request",
            serde_json::json!({
                "message": message_owned,
                "cookie": cookie_owned,
            }),
        );

        if let Some(window) = self.app_handle.get_webview_window("main") {
            let _ = window.show();
            let _ = window.set_focus();
        }

        eprintln!("[vasak-polkit] Return session path: {session_path}");
        session_path
    }

    async fn cancel_authentication(&mut self, session: ObjectPath<'_>) {
        eprintln!("[vasak-polkit] CancelAuthentication: {session}");

        let cookie = self
            .session_map
            .lock()
            .expect("lock session_map")
            .remove(session.as_str())
            .unwrap_or_default();

        self.pending.lock().expect("lock pending").remove(&cookie);

        let _ = self.app_handle.emit(
            "polkit-cancel",
            serde_json::json!({ "cookie": cookie }),
        );
    }
}

// ── Tauri Command ──────────────────────────────────────────────────

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
        .ok_or_else(|| format!("No pending auth for cookie: {cookie}"))?;

    tx.send(password).map_err(|_| "Receiver dropped".to_string())?;
    Ok(true)
}

// ── PAM authentication (runs on blocking thread) ───────────────────

async fn authenticate_pam(password: &str) -> bool {
    let pwd = password.to_string();
    let login = std::env::var("USER").unwrap_or_else(|_| "root".to_string());
    tokio::task::spawn_blocking(move || {
        let mut client = match pam::Client::with_password("polkit-1") {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[vasak-polkit] PAM Client::with_password failed: {e}");
                return false;
            }
        };
        client
            .conversation_mut()
            .set_credentials(&login, &pwd);
        let result = client.authenticate();
        if result.is_err() {
            eprintln!("[vasak-polkit] PAM authenticate failed: {:?}", result);
        }
        result.is_ok()
    })
    .await
    .unwrap_or(false)
}

// ── D-Bus agent registration ──────────────────────────────────────

async fn register_polkit_agent(
    app_handle: AppHandle,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<String>>>>,
    session_map: Arc<Mutex<HashMap<String, String>>>,
) {
    let conn = zbus::Connection::system()
        .await
        .expect("Failed to connect to system bus");

    let agent = PolkitAgent {
        app_handle: app_handle.clone(),
        pending,
        session_map,
        conn: conn.clone(),
    };

    conn.object_server()
        .at(
            "/org/freedesktop/PolicyKit1/AuthenticationAgent",
            agent,
        )
        .await
        .expect("Failed to register agent object on D-Bus");

    // Register a debug interface on a public path to test D-Bus dispatch
    struct DebugAgent;
    #[interface(name = "org.vasak.DebugAgent")]
    impl DebugAgent {
        async fn ping(&mut self) -> String {
            eprintln!("[vasak-polkit] Debug PING received");
            "pong".to_string()
        }
    }
    conn.object_server()
        .at("/org/vasak/DebugAgent", DebugAgent)
        .await
        .expect("Failed to register debug agent");

    eprintln!("[vasak-polkit] Bus unique name: {}", conn.unique_name().map(|n| n.as_str()).unwrap_or("?"));

    let sid = std::env::var("XDG_SESSION_ID").unwrap_or_else(|_| "1".to_string());
    eprintln!("[vasak-polkit] Registering agent for session {sid}");

    let subject: (&str, HashMap<String, Value<'_>>) = (
        "unix-session",
        HashMap::from([(
            "session-id".to_string(),
            Value::Str(sid.into()),
        )]),
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
        Ok(_) => eprintln!("[vasak-polkit] Registered successfully"),
        Err(e) => eprintln!("[vasak-polkit] Register failed (may already exist): {e}"),
    }

    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(3600)).await;
    }
}

// ── Application entry point ────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Capture zbus tracing logs to diagnose D-Bus dispatch issues
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_new("zbus=trace,zvariant=trace")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("zbus=debug")),
        )
        .try_init();

    let pending = Arc::new(Mutex::new(HashMap::new()));
    let session_map = Arc::new(Mutex::new(HashMap::new()));

    tauri::Builder::default()
        .manage(AppState {
            pending: pending.clone(),
        })
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_config_manager::init())
        .plugin(tauri_plugin_vicons::init())
        .setup(move |app| {
            let app_handle = app.handle().clone();
            let p = pending.clone();
            let sm = session_map.clone();

            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new()
                    .expect("Failed to create Tokio runtime for D-Bus agent");
                rt.block_on(async {
                    register_polkit_agent(app_handle, p, sm).await;
                });
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![submit_password])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
