//! Que los catálogos de idioma sirvan.
//!
//! El plugin de i18n los parsea en tiempo de ejecución y **paniquea** si no
//! puede, así que un error de sintaxis no se ve hasta que la aplicación no
//! arranca. Y una clave que falta en un idioma no falla: se muestra cruda, con el
//! nombre de la clave a la vista de la persona.
//!
//! En este diálogo eso es peor que en otros: aparece encima de lo que sea que la
//! persona esté haciendo, pidiéndole una contraseña. Una ventana así que dice
//! «unlock.prompt» no se parece a algo en lo que se deba escribir nada.

use std::collections::BTreeSet;
use std::path::PathBuf;

fn catalogo(idioma: &str) -> serde_yaml::Value {
    let ruta = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("locales")
        .join(format!("{idioma}.yml"));
    let texto = std::fs::read_to_string(&ruta)
        .unwrap_or_else(|e| panic!("no se pudo leer {}: {e}", ruta.display()));
    serde_yaml::from_str(&texto)
        .unwrap_or_else(|e| panic!("{} no es YAML válido: {e}", ruta.display()))
}

/// Todas las claves, aplanadas con puntos, como las busca el plugin.
fn claves(valor: &serde_yaml::Value, prefijo: &str, salida: &mut BTreeSet<String>) {
    match valor {
        serde_yaml::Value::Mapping(mapa) => {
            for (clave, hijo) in mapa {
                let nombre = clave.as_str().unwrap_or_default();
                let completa = if prefijo.is_empty() {
                    nombre.to_string()
                } else {
                    format!("{prefijo}.{nombre}")
                };
                claves(hijo, &completa, salida);
            }
        }
        _ => {
            salida.insert(prefijo.to_string());
        }
    }
}

fn claves_de(idioma: &str) -> BTreeSet<String> {
    let mut salida = BTreeSet::new();
    claves(&catalogo(idioma), "", &mut salida);
    salida
}

#[test]
fn los_dos_idiomas_parsean_y_la_raiz_es_un_mapeo() {
    for idioma in ["es", "en"] {
        assert!(
            catalogo(idioma).is_mapping(),
            "la raíz de {idioma}.yml tiene que ser un mapeo, no un valor suelto"
        );
    }
}

#[test]
fn los_dos_idiomas_tienen_las_mismas_claves() {
    let es = claves_de("es");
    let en = claves_de("en");

    let solo_es: Vec<_> = es.difference(&en).collect();
    let solo_en: Vec<_> = en.difference(&es).collect();

    assert!(
        solo_es.is_empty() && solo_en.is_empty(),
        "las claves no coinciden.\n  sólo en es: {solo_es:?}\n  sólo en en: {solo_en:?}"
    );
}

#[test]
fn ningun_texto_esta_vacio() {
    // Una clave vacía no es un texto faltante que se note: se muestra como nada,
    // y el control queda sin etiqueta.
    for idioma in ["es", "en"] {
        let raiz = catalogo(idioma);
        let mut todas = BTreeSet::new();
        claves(&raiz, "", &mut todas);

        let vacias: Vec<&String> = todas
            .iter()
            .filter(|clave| {
                let mut actual = &raiz;
                for parte in clave.split('.') {
                    actual = &actual[parte];
                }
                actual
                    .as_str()
                    .map(|s| s.trim().is_empty())
                    .unwrap_or(false)
            })
            .collect();

        assert!(
            vacias.is_empty(),
            "textos vacíos en {idioma}.yml: {vacias:?}"
        );
    }
}

#[test]
fn el_dialogo_del_disco_dice_de_cual() {
    // Sin el marcador, el diálogo pide una frase de paso sin decir para qué
    // disco. Es la única defensa que tiene la persona contra un pedido que no
    // esperaba: si no dice cuál, no hay nada que reconocer.
    for idioma in ["es", "en"] {
        let texto = catalogo(idioma)["unlock"]["prompt"]
            .as_str()
            .unwrap_or_default()
            .to_string();

        assert!(
            texto.contains("{0}"),
            "unlock.prompt de {idioma}.yml no lleva {{0}}: «{texto}»"
        );
    }
}

#[test]
fn las_dos_preguntas_no_se_llaman_igual() {
    // La contraseña de la cuenta y la frase de un disco son dos cosas distintas.
    // Que compartan la ventana no las hace la misma pregunta, y presentarlas con
    // el mismo título es lo que enseña a escribir la contraseña de la sesión
    // donde no va.
    for idioma in ["es", "en"] {
        let raiz = catalogo(idioma);
        let polkit = raiz["polkit"]["title"].as_str().unwrap_or_default();
        let desbloqueo = raiz["unlock"]["title"].as_str().unwrap_or_default();

        assert!(!polkit.is_empty() && !desbloqueo.is_empty());
        assert_ne!(
            polkit, desbloqueo,
            "los dos diálogos de {idioma}.yml se presentan igual"
        );
    }
}
