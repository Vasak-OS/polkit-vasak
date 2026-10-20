//! Que el afinado del WebKit siga cableado de punta a punta.
//!
//! El glue de GTK baja al `WebKitWebView` por el árbol de widgets y no se puede
//! ejercitar sin un display ni un WebKit reales, así que esta prueba no corre
//! GTK: lee el fuente y comprueba que las piezas sigan conectadas —el comando, sus
//! palancas, el registro en `lib.rs` y la llamada del frontend—. Si alguien borra
//! una de ellas, el afinado deja de aplicarse en silencio (la ventana sigue
//! andando, sólo consume de más), y sin esta guardia nadie se entera.

use std::path::PathBuf;

fn leer(ruta_relativa: &str) -> String {
    let ruta = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(ruta_relativa);
    std::fs::read_to_string(&ruta)
        .unwrap_or_else(|e| panic!("no se pudo leer {}: {e}", ruta.display()))
}

#[test]
fn el_comando_tiene_las_palancas_del_afinado() {
    let fuente = leer("src/commands/webkit.rs");
    assert!(
        fuente.contains("pub fn tune_webview"),
        "falta el comando tune_webview"
    );
    assert!(
        fuente.contains("set_enable_developer_extras(false)"),
        "falta apagar los extras de desarrollo"
    );
    assert!(
        fuente.contains("set_enable_back_forward_navigation_gestures(false)"),
        "falta apagar los gestos de navegación"
    );
    assert!(
        fuente.contains("CacheModel::DocumentViewer"),
        "falta el caché de visor de documento"
    );
}

#[test]
fn el_comando_esta_registrado_en_lib() {
    let fuente = leer("src/lib.rs");
    assert!(
        fuente.contains("commands::webkit::tune_webview"),
        "el comando no está en el generate_handler de lib.rs"
    );
}

#[test]
fn el_frontend_invoca_el_afinado_al_montar() {
    let fuente = leer("../src/App.vue");
    assert!(
        fuente.contains("invoke('tune_webview')"),
        "App.vue no llama a tune_webview"
    );
}
