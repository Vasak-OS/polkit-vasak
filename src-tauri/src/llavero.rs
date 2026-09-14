//! Recordar la frase de paso de un disco cifrado, en el llavero de la persona.
//!
//! ── Por qué se puede recordar ───────────────────────────────────────────────
//!
//! Un disco cifrado que se usa todos los días pide la frase todos los días, y a
//! eso la gente le encuentra la vuelta de la peor manera: eligiendo una frase
//! corta. Poder guardarla es lo que hace que valga la pena que sea larga.
//!
//! ── Por qué acá y no en un archivo propio ───────────────────────────────────
//!
//! `vasak-keyring` implementa el Secret Service, así que la frase queda donde
//! están las demás contraseñas de la sesión: cifrada con la contraseña maestra,
//! visible y borrable desde donde se miran los secretos. Un archivo propio sería
//! un segundo lugar donde buscar y un segundo lugar que auditar, y encima habría
//! que resolver de nuevo con qué se lo cifra.
//!
//! Todo lo de este módulo falla en silencio a propósito: no poder recordar la
//! frase no es motivo para no abrir el disco. Si el llavero está cerrado —o no
//! está—, se pide la frase como si nunca se hubiera guardado.

use std::collections::HashMap;

use zbus::Connection;
use zeroize::Zeroizing;
use zvariant::{OwnedObjectPath, OwnedValue, Value};

const DESTINO: &str = "org.freedesktop.secrets";
const RUTA_SERVICIO: &str = "/org/freedesktop/secrets";
const IFACE_SERVICIO: &str = "org.freedesktop.Secret.Service";
const IFACE_COLECCION: &str = "org.freedesktop.Secret.Collection";
const IFACE_ITEM: &str = "org.freedesktop.Secret.Item";
const IFACE_SESION: &str = "org.freedesktop.Secret.Session";

/// La colección por omisión de la persona.
const COLECCION: &str = "/org/freedesktop/secrets/aliases/default";

/// Con qué se marcan estos secretos para distinguirlos del resto del llavero.
///
/// `xdg:schema` es la convención de freedesktop para decir «de qué clase es esto»,
/// y la usan todas las implementaciones del Secret Service.
pub const ESQUEMA: &str = "ar.net.vasak.os.LuksPassphrase";

/// Un secreto del Secret Service: sesión, parámetros, valor y tipo.
type Secreto = (OwnedObjectPath, Vec<u8>, Vec<u8>, String);

/// Con qué se busca y se guarda la frase de un volumen.
///
/// La llave es el UUID del volumen y no el `/dev/sdX`: el mismo disco aparece
/// como `sdb1` o `sdc1` según en qué orden se hayan enchufado las cosas, y una
/// frase recordada que deja de encontrarse cuando cambia el orden no sirve para
/// nada.
pub fn atributos(uuid: &str) -> HashMap<String, String> {
    HashMap::from([
        ("xdg:schema".to_string(), ESQUEMA.to_string()),
        ("uuid".to_string(), uuid.to_string()),
    ])
}

/// Cómo se llama la entrada en la lista de contraseñas.
///
/// Se lee sin contexto —en una pantalla llena de secretos de todo tipo— así que
/// dice qué es y de qué disco, no sólo de qué disco.
pub fn etiqueta_de_entrada(nombre_del_disco: &str) -> String {
    format!("Disco cifrado: {nombre_del_disco}")
}

/// Abre una sesión con el llavero.
///
/// `plain` manda el secreto sin cifrar por el bus, que es lo mismo que hacen el
/// resto de las aplicaciones contra un llavero local: el bus de sesión es del
/// usuario y sólo él lo lee. El cifrado de transporte del Secret Service existe
/// para el caso remoto, que acá no se da.
async fn abrir_sesion(conexion: &Connection) -> Result<OwnedObjectPath, String> {
    let respuesta = conexion
        .call_method(
            Some(DESTINO),
            RUTA_SERVICIO,
            Some(IFACE_SERVICIO),
            "OpenSession",
            &("plain", Value::from("")),
        )
        .await
        .map_err(|error| format!("no se pudo abrir una sesión con el llavero: {error}"))?;

    let cuerpo = respuesta.body();
    let (_salida, sesion): (OwnedValue, OwnedObjectPath) = cuerpo
        .deserialize()
        .map_err(|error| format!("respuesta inesperada del llavero: {error}"))?;

    Ok(sesion)
}

/// Cierra la sesión. Que falle no cambia nada de lo que ya se hizo.
async fn cerrar_sesion(conexion: &Connection, sesion: &OwnedObjectPath) {
    let _ = conexion
        .call_method(Some(DESTINO), sesion, Some(IFACE_SESION), "Close", &())
        .await;
}

/// La frase guardada para ese volumen, si hay alguna y el llavero está abierto.
///
/// Un secreto que el llavero devuelve como bloqueado se ignora: desbloquearlo
/// exigiría pedir *otra* contraseña —la maestra— para evitar pedir esta, que es
/// justo al revés de lo que la casilla prometía.
pub async fn frase_guardada(conexion: &Connection, uuid: &str) -> Option<Zeroizing<String>> {
    if uuid.trim().is_empty() {
        return None;
    }

    let sesion = abrir_sesion(conexion).await.ok()?;
    let frase = leer_frase(conexion, &sesion, uuid).await;
    cerrar_sesion(conexion, &sesion).await;

    frase
}

async fn leer_frase(
    conexion: &Connection,
    sesion: &OwnedObjectPath,
    uuid: &str,
) -> Option<Zeroizing<String>> {
    let respuesta = conexion
        .call_method(
            Some(DESTINO),
            RUTA_SERVICIO,
            Some(IFACE_SERVICIO),
            "SearchItems",
            &(atributos(uuid),),
        )
        .await
        .ok()?;

    let cuerpo = respuesta.body();
    let (abiertos, _bloqueados): (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) =
        cuerpo.deserialize().ok()?;

    let item = abiertos.into_iter().next()?;

    let respuesta = conexion
        .call_method(
            Some(DESTINO),
            &item,
            Some(IFACE_ITEM),
            "GetSecret",
            &(sesion,),
        )
        .await
        .ok()?;

    let cuerpo = respuesta.body();
    let (_sesion, _parametros, valor, _tipo): Secreto = cuerpo.deserialize().ok()?;

    frase_desde_bytes(&valor)
}

/// Una frase de paso es texto, pero el llavero guarda bytes.
///
/// Una entrada que no sea UTF-8 se descarta en vez de repararse: mandarle a
/// `cryptsetup` una frase con caracteres cambiados da «frase incorrecta» sin
/// explicación, y es mejor pedirla de nuevo.
pub fn frase_desde_bytes(bytes: &[u8]) -> Option<Zeroizing<String>> {
    if bytes.is_empty() {
        return None;
    }

    String::from_utf8(bytes.to_vec()).ok().map(Zeroizing::new)
}

/// Guarda la frase, reemplazando la que hubiera para ese mismo volumen.
///
/// `replace` en verdadero y no en falso: si no, cambiar la frase de un disco
/// dejaría dos entradas con los mismos atributos y la búsqueda devolvería
/// cualquiera de las dos.
pub async fn recordar_frase(
    conexion: &Connection,
    uuid: &str,
    nombre_del_disco: &str,
    frase: &str,
) -> Result<(), String> {
    if uuid.trim().is_empty() {
        return Err("el volumen no tiene UUID, así que no hay con qué recordarlo".to_string());
    }

    let sesion = abrir_sesion(conexion).await?;
    let resultado = escribir_frase(conexion, &sesion, uuid, nombre_del_disco, frase).await;
    cerrar_sesion(conexion, &sesion).await;

    resultado
}

async fn escribir_frase(
    conexion: &Connection,
    sesion: &OwnedObjectPath,
    uuid: &str,
    nombre_del_disco: &str,
    frase: &str,
) -> Result<(), String> {
    let propiedades: HashMap<&str, Value<'_>> = HashMap::from([
        (
            "org.freedesktop.Secret.Item.Label",
            Value::from(etiqueta_de_entrada(nombre_del_disco)),
        ),
        (
            "org.freedesktop.Secret.Item.Attributes",
            Value::from(atributos(uuid)),
        ),
    ]);

    let secreto: Secreto = (
        sesion.clone(),
        Vec::new(),
        frase.as_bytes().to_vec(),
        "text/plain".to_string(),
    );

    conexion
        .call_method(
            Some(DESTINO),
            COLECCION,
            Some(IFACE_COLECCION),
            "CreateItem",
            &(propiedades, secreto, true),
        )
        .await
        .map_err(|error| format!("el llavero no guardó la frase: {error}"))?;

    Ok(())
}

/// Olvida la frase de un volumen.
///
/// Se llama cuando la frase guardada resultó no abrir el disco: alguien la
/// cambió con `cryptsetup`, y dejarla ahí sólo garantiza un intento fallido cada
/// vez que se enchufe el disco.
pub async fn olvidar_frase(conexion: &Connection, uuid: &str) {
    if uuid.trim().is_empty() {
        return;
    }

    let Ok(respuesta) = conexion
        .call_method(
            Some(DESTINO),
            RUTA_SERVICIO,
            Some(IFACE_SERVICIO),
            "SearchItems",
            &(atributos(uuid),),
        )
        .await
    else {
        return;
    };

    let cuerpo = respuesta.body();
    let Ok((abiertos, bloqueados)) =
        cuerpo.deserialize::<(Vec<OwnedObjectPath>, Vec<OwnedObjectPath>)>()
    else {
        return;
    };

    for item in abiertos.into_iter().chain(bloqueados) {
        let _ = conexion
            .call_method(Some(DESTINO), &item, Some(IFACE_ITEM), "Delete", &())
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_frase_se_indexa_por_uuid_y_no_por_dispositivo() {
        // El mismo disco es /dev/sdb1 hoy y /dev/sdc1 mañana; el UUID no cambia.
        let atributos = atributos("a1b2-c3d4");

        assert_eq!(atributos.get("uuid").map(String::as_str), Some("a1b2-c3d4"));
        assert_eq!(
            atributos.get("xdg:schema").map(String::as_str),
            Some(ESQUEMA)
        );
        assert_eq!(atributos.len(), 2);
    }

    #[test]
    fn el_esquema_distingue_estos_secretos_del_resto_del_llavero() {
        // Sin él, una búsqueda por UUID podría traer el secreto de otra cosa que
        // casualmente guarde un atributo con ese nombre.
        assert!(ESQUEMA.starts_with("ar.net.vasak.os."));
    }

    #[test]
    fn la_entrada_dice_de_que_es_y_no_solo_de_cual() {
        let etiqueta = etiqueta_de_entrada("Respaldos");

        assert!(etiqueta.contains("Respaldos"));
        assert!(etiqueta.to_lowercase().contains("cifrado"));
    }

    #[test]
    fn una_frase_vacia_en_el_llavero_no_cuenta_como_frase() {
        // Guardarla vacía no debería pasar, pero si pasa, intentar abrir el disco
        // con nada gasta un intento y confunde el diagnóstico.
        assert!(frase_desde_bytes(b"").is_none());
    }

    #[test]
    fn una_frase_que_no_es_utf8_se_descarta() {
        assert!(frase_desde_bytes(&[0xff, 0xfe, 0x00]).is_none());
    }

    #[test]
    fn una_frase_normal_vuelve_entera() {
        let frase = frase_desde_bytes("ñandú correcaminos".as_bytes()).unwrap();

        assert_eq!(frase.as_str(), "ñandú correcaminos");
    }
}
