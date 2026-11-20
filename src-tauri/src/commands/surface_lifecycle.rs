//! Matar el `WebKitWebProcess` de la ventana del diálogo cuando está escondida,
//! para liberar su RAM (~240 MB), y reponerlo recargando al volver a mostrarla.
//!
//! La ventana `main` es única y residente: se construye escondida al arrancar y
//! sólo se muestra durante un pedido de polkit o de desbloqueo. Entre pedidos
//! —casi toda la sesión— su proceso de WebKit seguía vivo sin hacer nada. Acá se
//! programa matarlo tras una gracia escondida; al mostrarla de nuevo se recarga
//! la página y WebKit vuelve a levantar el proceso.
//!
//! **No** se destruye la ventana: destruir un webview deja colgado su proceso
//! bajo wry/GTK. `terminate_web_process()` lo baja de verdad sin tocar la ventana.
//!
//! Como al mostrar se recarga, el pedido en vuelo se perdería: su evento
//! (`polkit-request` / `unlock-request`) se emite igual para el caso en que el
//! proceso seguía vivo, pero además el backend guarda qué debería estar en
//! pantalla ([`set_polkit_view`] / [`set_unlock_view`]) y la página lo vuelve a
//! pedir al montar ([`polkit_pending`] / [`unlock_pending`]). La página, tras
//! recargar, remonta y pregunta; el evento perdido no importa.
//!
//! Vive en `commands/`, excluido de cobertura como `webkit.rs`: baja al webview
//! por GTK y no se puede montar sin un display real. La única parte pura
//! —[`reload_needed`]— va con su prueba acá abajo.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Manager};

/// Cuánto espera la ventana escondida antes de matar su proceso. Corto: la
/// ventana no se usa casi nunca, así que se libera a los pocos segundos de cada
/// cierre y del arranque.
const GRACE: Duration = Duration::from_secs(5);

/// Si el proceso se mató y, por eso, hay que recargar antes de mostrar.
static TERMINATED: AtomicBool = AtomicBool::new(false);

/// Generación de la muerte pendiente. Rearmar o cancelar la incrementa; la tarea
/// dormida, y el cierre que corre en el hilo principal, no matan si la encuentran
/// distinta. Es un temporizador cancelable sin un `SourceId` de glib, que no
/// cruza hilos.
static GEN: AtomicU64 = AtomicU64::new(0);

/// Lo que el backend quiere mostrar en la ventana. La página lo pide al montar,
/// porque al recargar pierde el evento que lo traía.
static POLKIT_VIEW: Mutex<Option<PolkitView>> = Mutex::new(None);
static UNLOCK_VIEW: Mutex<Option<UnlockView>> = Mutex::new(None);

/// Las pruebas que escriben `POLKIT_VIEW` (la de este módulo y las del agente en
/// `lib.rs`, que pasan por `begin_authentication`) corren en paralelo sobre un
/// estático global: la que lo toma las serializa.
#[cfg(test)]
static VIEWS_TEST_GUARD: Mutex<()> = Mutex::new(());

#[cfg(test)]
pub(crate) fn lock_views_for_tests() -> std::sync::MutexGuard<'static, ()> {
    VIEWS_TEST_GUARD.lock().unwrap_or_else(|e| e.into_inner())
}

/// El pedido de polkit a la vista: el mismo contenido que el evento
/// `polkit-request`.
#[derive(Clone, Serialize)]
pub struct PolkitView {
    pub message: String,
    pub cookie: String,
}

/// El pedido de desbloqueo a la vista: el mismo contenido que el evento
/// `unlock-request`.
#[derive(Clone, Serialize)]
pub struct UnlockView {
    pub id: String,
    pub name: String,
    #[serde(rename = "wrongPassphrase")]
    pub wrong_passphrase: bool,
}

/// Decide, pura, si mostrar necesita recargar: sólo cuando el proceso se mató.
///
/// Trivial, pero es la regla que sostiene todo lo demás y la única parte de este
/// módulo que se puede probar sin un display.
pub fn reload_needed(terminated: bool) -> bool {
    terminated
}

/// Programa matar el proceso tras [`GRACE`] escondida. Reemplaza cualquier muerte
/// pendiente. Seguro desde cualquier hilo.
///
/// Genérica en el runtime —como el agente— para que las pruebas usen el de
/// mentira de Tauri; en producción siempre es Wry.
pub fn arm_after_hide<R: tauri::Runtime>(app: &AppHandle<R>) {
    let generation = GEN.fetch_add(1, Ordering::SeqCst) + 1;
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(GRACE).await;
        maybe_terminate(&app, generation);
    });
}

/// Cancela la muerte pendiente. Se llama al ir a mostrar: incrementar la
/// generación invalida tanto la tarea dormida como un cierre ya encolado en el
/// hilo principal.
pub fn cancel<R: tauri::Runtime>(_app: &AppHandle<R>) {
    GEN.fetch_add(1, Ordering::SeqCst);
}

/// Deja la ventana lista para mostrarse: cancela la muerte pendiente y, si el
/// proceso ya se había matado, recarga para reponerlo. Hay que llamarlo con el
/// `*_view` ya puesto, para que la página recargada lo encuentre al preguntar.
///
/// `cancel` sube la generación **antes** de encolar nada, así cualquier muerte
/// pendiente —dormida o ya en la cola del hilo principal— ve la generación nueva
/// y no mata. La decisión de recargar va **en el hilo principal**, serializada
/// con [`maybe_terminate`]: si quedara en este hilo, entre leer `TERMINATED` y
/// que una muerte en el hilo principal lo pusiera en `true` habría una ventana en
/// la que la página se mostraría con el proceso muerto y sin recargar. En el hilo
/// principal, o la muerte corrió antes (`TERMINATED` en `true` → recarga) o este
/// closure corrió antes (la generación ya subió y la muerte se saltea).
pub fn prepare_show<R: tauri::Runtime>(app: &AppHandle<R>) {
    cancel(app);
    let app_main = app.clone();
    let _ = app.run_on_main_thread(move || {
        if reload_needed(TERMINATED.swap(false, Ordering::SeqCst)) {
            reload_on_main(&app_main);
        }
    });
}

/// Guarda —o limpia— qué pedido de polkit debería estar en pantalla.
pub fn set_polkit_view(view: Option<PolkitView>) {
    if let Ok(mut guard) = POLKIT_VIEW.lock() {
        *guard = view;
    }
}

/// Guarda —o limpia— qué pedido de desbloqueo debería estar en pantalla.
pub fn set_unlock_view(view: Option<UnlockView>) {
    if let Ok(mut guard) = UNLOCK_VIEW.lock() {
        *guard = view;
    }
}

/// Qué pedido de polkit mostrar, o `None`. La página lo pide al montar, porque al
/// recargar pierde el `polkit-request` que lo traía.
#[tauri::command]
pub fn polkit_pending() -> Option<PolkitView> {
    POLKIT_VIEW.lock().ok().and_then(|guard| guard.clone())
}

/// Qué pedido de desbloqueo mostrar, o `None`. Igual que [`polkit_pending`].
#[tauri::command]
pub fn unlock_pending() -> Option<UnlockView> {
    UNLOCK_VIEW.lock().ok().and_then(|guard| guard.clone())
}

/// Mata el proceso si la muerte sigue vigente. Corre en el hilo principal de GTK
/// y vuelve a mirar la generación ahí: si un `cancel` (un show) la cambió
/// mientras dormía o mientras esperaba en la cola, no mata. Así un show nunca
/// deja la ventana con el proceso muerto sin recargar.
fn maybe_terminate<R: tauri::Runtime>(app: &AppHandle<R>, generation: u64) {
    let app_main = app.clone();
    let _ = app.run_on_main_thread(move || {
        if GEN.load(Ordering::SeqCst) != generation {
            return;
        }
        if let Some(window) = app_main.get_webview_window("main") {
            // Un `show` que entró en el hueco entre `hide()` y `arm_after_hide`
            // (otro pedido justo al cerrarse el anterior) rearma la generación a
            // este mismo valor, así que la guarda de arriba no lo ataja: la
            // ventana está visible y en uso. No se mata; el próximo `hide` rearma.
            if window.is_visible().unwrap_or(false) {
                return;
            }
            let _ = window.with_webview(|platform| {
                use webkit2gtk::WebViewExt;
                platform.inner().terminate_web_process();
            });
            TERMINATED.store(true, Ordering::SeqCst);
        }
    });
}

/// Recarga la ventana, con lo que WebKit respawnea el proceso que
/// [`maybe_terminate`] mató. Ya corre en el hilo principal: lo llama el closure
/// de [`prepare_show`], que [`AppHandle::run_on_main_thread`] despacha ahí.
fn reload_on_main<R: tauri::Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.with_webview(|platform| {
            use webkit2gtk::WebViewExt;
            platform.inner().reload();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solo_recarga_si_el_proceso_se_mato() {
        assert!(reload_needed(true));
        assert!(!reload_needed(false));
    }

    // Los almacenes son globales: una sola prueba los recorre en orden y toma la
    // guarda que comparten las pruebas del agente, para que no se pisen.
    #[test]
    fn la_vista_guardada_es_la_que_devuelven_los_comandos_de_consulta() {
        let _vistas = lock_views_for_tests();
        set_polkit_view(Some(PolkitView {
            message: "Instalar".into(),
            cookie: "c1".into(),
        }));
        let polkit = polkit_pending().expect("hay un pedido de polkit guardado");
        assert_eq!(polkit.message, "Instalar");
        assert_eq!(polkit.cookie, "c1");
        set_polkit_view(None);
        assert!(polkit_pending().is_none());

        set_unlock_view(Some(UnlockView {
            id: "d1".into(),
            name: "Respaldo".into(),
            wrong_passphrase: true,
        }));
        let unlock = unlock_pending().expect("hay un desbloqueo guardado");
        assert_eq!(unlock.id, "d1");
        assert_eq!(unlock.name, "Respaldo");
        assert!(unlock.wrong_passphrase);
        set_unlock_view(None);
        assert!(unlock_pending().is_none());
    }

    #[test]
    fn el_pedido_de_desbloqueo_se_serializa_con_wrong_passphrase_en_camel_case() {
        let json = serde_json::to_value(UnlockView {
            id: "d".into(),
            name: "n".into(),
            wrong_passphrase: false,
        })
        .unwrap();
        assert_eq!(json["wrongPassphrase"], false);
    }
}
