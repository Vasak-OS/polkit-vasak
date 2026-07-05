use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::oneshot;
use zbus::interface;
use zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

struct AppState {
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<String>>>>,
}

static SESSION_COUNTER: AtomicU64 = AtomicU64::new(1);

struct PolkitAgent {
    app_handle: AppHandle,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<String>>>>,
    session_map: Arc<Mutex<HashMap<String, String>>>,
}

#[interface(name = "org.freedesktop.PolicyKit1.AuthenticationAgent")]
impl PolkitAgent {
    async fn begin_authentication(
        &mut self,
        action_id: &str,
        message: &str,
        _icon_name: &str,
        details: HashMap<String, String>,
        cookie: &str,
        identities: Vec<(String, HashMap<String, OwnedValue>)>,
    ) -> OwnedObjectPath {
        eprintln!("[vasak-polkit] begin_authentication action_id={action_id} cookie={cookie}");

        let cookie_owned = cookie.to_string();
        let message_owned = message.to_string();
        let action_id_owned = action_id.to_string();
        let subject_pid: u32 = details
            .get("polkit.subject-pid")
            .and_then(|p| p.parse().ok())
            .unwrap_or(0);
        let identity = identities.into_iter().next();
        let (identity_kind, identity_details) = identity
            .unwrap_or_else(|| ("unix-user".to_string(), HashMap::new()));
        let identity_uid: u32 = identity_details
            .get("uid")
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(1000);

        let session_num = SESSION_COUNTER.fetch_add(1, Ordering::Relaxed);
        let session_path_str =
            format!("/org/freedesktop/PolicyKit1/AuthenticationAgent/Session/{session_num}");
        let session_path: OwnedObjectPath = session_path_str
            .try_into()
            .expect("Static session path should always be valid");

        let (password_tx, password_rx) = oneshot::channel::<String>();
        let (done_tx, done_rx) = oneshot::channel::<bool>();

        self.pending
            .lock()
            .expect("lock pending")
            .insert(cookie_owned.clone(), password_tx);

        self.session_map
            .lock()
            .expect("lock session_map")
            .insert(session_path.to_string(), cookie_owned.clone());

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

            if !authenticate_pam(&password).await {
                let _ = done_tx.send(false);
                return;
            }

            let ok = call_authentication_response_via_sudo(
                password,
                cookie_task,
                identity_kind_task,
                identity_uid,
                subject_pid_task,
            )
            .await;

            let _ = done_tx.send(ok);
        });

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

        session_path
    }

    async fn send_password(
        &mut self,
        cookie: &str,
        password: &str,
    ) -> bool {
        let tx = self
            .pending
            .lock()
            .expect("lock pending")
            .remove(cookie);
        match tx {
            Some(tx) => tx.send(password.to_string()).is_ok(),
            None => {
                eprintln!("[vasak-polkit] No pending auth for cookie={cookie}");
                false
            }
        }
    }

    async fn cancel_authentication(&mut self, session: ObjectPath<'_>) {
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

#[tauri::command]
async fn cancel_pending(
    state: State<'_, AppState>,
    cookie: String,
) -> Result<(), String> {
    state.pending
        .lock()
        .map_err(|e| e.to_string())?
        .remove(&cookie);
    Ok(())
}

async fn call_authentication_response_via_sudo(
    password: String,
    cookie: String,
    identity_kind: String,
    uid: u32,
    subject_pid: u32,
) -> bool {
    let result = tokio::task::spawn_blocking(move || -> Result<bool, String> {
        let helper_path = {
            let mut p = std::env::current_exe()
                .map_err(|e| format!("current_exe: {e}"))?;
            p.set_file_name("polkit-agent-helper-dbus");
            if !p.exists() {
                let mut dev = std::env::current_exe()
                    .map_err(|e| format!("current_exe: {e}"))?;
                dev.pop();
                dev.push("polkit-agent-helper-dbus");
                dev
            } else {
                p
            }
        };

        let mut child = Command::new("sudo")
            .arg("-S")
            .arg(&helper_path)
            .arg("--cookie")
            .arg(&cookie)
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
            .map_err(|e| format!("spawn sudo: {e}"))?;

        if let Some(ref mut stdin) = child.stdin {
            use std::io::Write;
            stdin
                .write_all(format!("{password}\n").as_bytes())
                .map_err(|e| format!("write password: {e}"))?;
        }
        let output = child
            .wait_with_output()
            .map_err(|e| format!("wait sudo: {e}"))?;

        if output.status.success() {
            Ok(true)
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            Err(format!("sudo failed: {stderr}"))
        }
    })
    .await;

    match result {
        Ok(Ok(ok)) => ok,
        Ok(Err(e)) => {
            eprintln!("[vasak-polkit] sudo error: {e}");
            false
        }
        Err(e) => {
            eprintln!("[vasak-polkit] sudo panic: {e}");
            false
        }
    }
}

async fn authenticate_pam(password: &str) -> bool {
    let pwd = password.to_string();
    let login = std::env::var("USER").unwrap_or_else(|_| "root".to_string());
    tokio::task::spawn_blocking(move || {
        let mut client = match pam::Client::with_password("polkit-1") {
            Ok(c) => c,
            Err(_) => return false,
        };
        client
            .conversation_mut()
            .set_credentials(&login, &pwd);
        client.authenticate().is_ok()
    })
    .await
    .unwrap_or(false)
}

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
        pending: pending.clone(),
        session_map,
    };

    conn.object_server()
        .at(
            "/org/freedesktop/PolicyKit1/AuthenticationAgent",
            agent,
        )
        .await
        .expect("Failed to register agent object on D-Bus");

    eprintln!("[vasak-polkit] Bus unique name: {}", conn.unique_name().map(|n| n.as_str()).unwrap_or("?"));

    let uid = unsafe { libc::getuid() };
    let subject: (&str, HashMap<String, Value<'_>>) = (
        "unix-user",
        HashMap::from([(
            "uid".to_string(),
            Value::U32(uid),
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
        Err(e) => eprintln!("[vasak-polkit] Register failed: {e}"),
    }

    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(3600)).await;
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
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
        .invoke_handler(tauri::generate_handler![submit_password, cancel_pending])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
