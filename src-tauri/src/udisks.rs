//! Lo mínimo de `udisks2` para abrir un disco cifrado y montarlo.
//!
//! ── Por qué D-Bus y no `udisksctl` ──────────────────────────────────────────
//!
//! `udisksctl` registra su propio agente de autenticación de texto cuando su
//! entrada estándar es una terminal, y pide la frase de paso con un `Passphrase:`
//! que nadie va a ver desde una aplicación de escritorio. Por el bus se habla con
//! el mismo servicio sin ese rodeo, y el error llega con su nombre
//! —`NotAuthorizedDismissed`, `AlreadyMounted`— en vez de un código de salida y
//! un texto en inglés que habría que adivinar.

use std::collections::HashMap;

use zbus::Connection;
use zvariant::{OwnedObjectPath, OwnedValue, Value};

const DESTINO: &str = "org.freedesktop.UDisks2";
const RUTA_GESTOR: &str = "/org/freedesktop/UDisks2/Manager";
const IFACE_GESTOR: &str = "org.freedesktop.UDisks2.Manager";
const IFACE_BLOQUE: &str = "org.freedesktop.UDisks2.Block";
const IFACE_CIFRADO: &str = "org.freedesktop.UDisks2.Encrypted";
const IFACE_ARCHIVOS: &str = "org.freedesktop.UDisks2.Filesystem";

/// Lo que hace falta saber de una partición antes de pedir nada.
#[derive(Debug, Clone)]
pub struct Bloque {
    pub objeto: OwnedObjectPath,
    /// El identificador del volumen. Es con lo que se recuerda la frase: sobrevive
    /// a que el disco cambie de `/dev/sdb1` a `/dev/sdc1` entre dos enchufadas.
    pub uuid: String,
    /// La etiqueta del volumen, si tiene. Es lo que se le muestra a la persona.
    pub etiqueta: String,
}

impl Bloque {
    /// Cómo nombrarlo en el diálogo.
    ///
    /// La etiqueta es lo que la persona reconoce; sin ella queda el nombre del
    /// dispositivo, que al menos es el que figura en la ventana desde donde se
    /// hizo clic. Nunca queda vacío: un diálogo que pide una contraseña sin decir
    /// para qué disco es exactamente lo que no se puede permitir.
    pub fn nombre_visible(&self, dispositivo: &str) -> String {
        if self.etiqueta.trim().is_empty() {
            dispositivo.to_string()
        } else {
            self.etiqueta.clone()
        }
    }
}

/// Si una ruta puede ser un dispositivo de bloque.
///
/// Esto no es una comprobación de seguridad —quien llama ya puede hablar con
/// `udisks2` por su cuenta— sino la manera de no mandarle al servicio cualquier
/// cosa que llegue por el bus. `ResolveDevice` con una ruta arbitraria contesta
/// «no existe», que es un error peor de leer que este.
pub fn ruta_de_dispositivo_valida(ruta: &str) -> bool {
    ruta.starts_with("/dev/")
        && ruta.len() > "/dev/".len()
        && !ruta.contains("..")
        && !ruta.contains('\0')
}

/// Traduce `/dev/sda3` al objeto que `udisks2` usa para esa partición.
///
/// Se pregunta en vez de armar la ruta a mano: la codificación de un nombre de
/// dispositivo en una ruta de objeto tiene sus reglas —los caracteres que no son
/// alfanuméricos van a `_XX`— y no hay razón para reimplementarlas.
pub async fn resolver_dispositivo(
    conexion: &Connection,
    dispositivo: &str,
) -> Result<OwnedObjectPath, String> {
    if !ruta_de_dispositivo_valida(dispositivo) {
        return Err(format!("«{dispositivo}» no es la ruta de un dispositivo"));
    }

    let especificacion: HashMap<&str, Value<'_>> =
        HashMap::from([("path", Value::from(dispositivo))]);
    let opciones: HashMap<&str, Value<'_>> = HashMap::new();

    let respuesta = conexion
        .call_method(
            Some(DESTINO),
            RUTA_GESTOR,
            Some(IFACE_GESTOR),
            "ResolveDevice",
            &(especificacion, opciones),
        )
        .await
        .map_err(|error| format!("no se pudo consultar el dispositivo: {error}"))?;

    let cuerpo = respuesta.body();
    let objetos: Vec<OwnedObjectPath> = cuerpo
        .deserialize()
        .map_err(|error| format!("respuesta inesperada de udisks2: {error}"))?;

    objetos
        .into_iter()
        .next()
        .ok_or_else(|| format!("udisks2 no conoce {dispositivo}"))
}

/// Una propiedad de un objeto de `udisks2`.
async fn propiedad(
    conexion: &Connection,
    objeto: &OwnedObjectPath,
    interfaz: &str,
    nombre: &str,
) -> Result<OwnedValue, String> {
    let respuesta = conexion
        .call_method(
            Some(DESTINO),
            objeto,
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &(interfaz, nombre),
        )
        .await
        .map_err(|error| format!("{interfaz}.{nombre}: {error}"))?;

    let cuerpo = respuesta.body();
    cuerpo
        .deserialize::<OwnedValue>()
        .map_err(|error| format!("{interfaz}.{nombre}: respuesta inesperada: {error}"))
}

/// Una propiedad de texto, o vacío si no se pudo leer.
///
/// Un volumen sin etiqueta y uno cuya interfaz no está no se distinguen acá a
/// propósito: las dos cosas significan lo mismo para quien las usa —no hay dato—
/// y quien necesita saber si está cifrado pregunta `estado_de_cifrado`.
async fn propiedad_de_texto(
    conexion: &Connection,
    objeto: &OwnedObjectPath,
    interfaz: &str,
    nombre: &str,
) -> String {
    match propiedad(conexion, objeto, interfaz, nombre).await {
        Ok(valor) => String::try_from(valor).unwrap_or_default(),
        Err(_) => String::new(),
    }
}

/// Lo que `udisks2` sabe de una partición.
pub async fn describir(conexion: &Connection, dispositivo: &str) -> Result<Bloque, String> {
    let objeto = resolver_dispositivo(conexion, dispositivo).await?;

    Ok(Bloque {
        uuid: propiedad_de_texto(conexion, &objeto, IFACE_BLOQUE, "IdUUID").await,
        etiqueta: propiedad_de_texto(conexion, &objeto, IFACE_BLOQUE, "IdLabel").await,
        objeto,
    })
}

/// En qué estado está la parte cifrada de un volumen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EstadoDeCifrado {
    /// No es un volumen cifrado: se monta y ya.
    Ninguno,
    /// Es cifrado y está cerrado: hace falta la frase.
    Bloqueado,
    /// Es cifrado y ya está abierto: queda montar el volumen en claro.
    Abierto(OwnedObjectPath),
}

/// Si el volumen está cifrado, y si ya está abierto.
///
/// La pregunta se le hace a la interfaz `Encrypted` y no a `IdType`: el tipo
/// sirve para mostrar, pero quien decide si hay algo que desbloquear es
/// `udisks2`. Un volumen sin esa interfaz no contesta la propiedad, y eso es
/// exactamente «no está cifrado».
///
/// `udisks2` devuelve `/` en `CleartextDevice` cuando el volumen está cerrado:
/// no es una ruta de objeto útil sino su manera de decir «ninguno».
pub async fn estado_de_cifrado(
    conexion: &Connection,
    volumen: &OwnedObjectPath,
) -> EstadoDeCifrado {
    let Ok(valor) = propiedad(conexion, volumen, IFACE_CIFRADO, "CleartextDevice").await else {
        return EstadoDeCifrado::Ninguno;
    };

    match OwnedObjectPath::try_from(valor) {
        Ok(ruta) if ruta.as_str() != "/" => EstadoDeCifrado::Abierto(ruta),
        Ok(_) => EstadoDeCifrado::Bloqueado,
        // La interfaz está, así que es cifrado; lo que no se pudo es leer dónde
        // quedó abierto. Pedir la frase es la salida segura: `Unlock` sobre algo
        // ya abierto contesta que lo está, y eso se maneja.
        Err(_) => EstadoDeCifrado::Bloqueado,
    }
}

/// Abre el volumen cifrado con la frase, y devuelve el volumen en claro.
pub async fn desbloquear(
    conexion: &Connection,
    cifrado: &OwnedObjectPath,
    frase: &str,
) -> Result<OwnedObjectPath, ErrorDeUdisks> {
    let opciones: HashMap<&str, Value<'_>> = HashMap::new();

    let respuesta = conexion
        .call_method(
            Some(DESTINO),
            cifrado,
            Some(IFACE_CIFRADO),
            "Unlock",
            &(frase, opciones),
        )
        .await
        .map_err(ErrorDeUdisks::de_zbus)?;

    let cuerpo = respuesta.body();
    cuerpo
        .deserialize::<OwnedObjectPath>()
        .map_err(|error| ErrorDeUdisks::definitivo(format!("respuesta inesperada: {error}")))
}

/// Monta el volumen y devuelve dónde quedó.
///
/// Si ya estaba montado no es un fallo: se devuelve el punto de montaje que
/// tiene. Quien llamó quiere llegar a los archivos, y ya se puede.
pub async fn montar(
    conexion: &Connection,
    volumen: &OwnedObjectPath,
) -> Result<String, ErrorDeUdisks> {
    let opciones: HashMap<&str, Value<'_>> = HashMap::new();

    match conexion
        .call_method(
            Some(DESTINO),
            volumen,
            Some(IFACE_ARCHIVOS),
            "Mount",
            &(opciones,),
        )
        .await
    {
        Ok(respuesta) => {
            let cuerpo = respuesta.body();
            cuerpo.deserialize::<String>().map_err(|error| {
                ErrorDeUdisks::definitivo(format!("respuesta inesperada: {error}"))
            })
        }
        Err(error) => {
            let error = ErrorDeUdisks::de_zbus(error);
            if error.clase != ClaseDeError::YaMontado {
                return Err(error);
            }
            punto_de_montaje(conexion, volumen).await.ok_or(error)
        }
    }
}

/// Cuánto se espera a que aparezca el sistema de archivos de un volumen recién
/// abierto, y cada cuánto se vuelve a mirar.
///
/// `Unlock` devuelve el objeto del volumen en claro apenas lo crea, pero la
/// interfaz `Filesystem` la agrega `udisks2` después de sondear el contenido.
/// Montar en el instante siguiente falla con «no existe ese método» y parecería
/// que el disco no tiene sistema de archivos, cuando lo único que pasó es que se
/// preguntó demasiado pronto.
const ESPERA_TOTAL: std::time::Duration = std::time::Duration::from_secs(5);
const ESPERA_ENTRE_INTENTOS: std::time::Duration = std::time::Duration::from_millis(100);

/// Monta el volumen, esperando a que `udisks2` termine de reconocerlo.
pub async fn montar_cuando_este_listo(
    conexion: &Connection,
    volumen: &OwnedObjectPath,
) -> Result<String, ErrorDeUdisks> {
    let limite = std::time::Instant::now() + ESPERA_TOTAL;

    loop {
        match montar(conexion, volumen).await {
            Ok(punto) => return Ok(punto),
            Err(error)
                if error.clase == ClaseDeError::TodaviaNoListo
                    && std::time::Instant::now() < limite =>
            {
                tokio::time::sleep(ESPERA_ENTRE_INTENTOS).await;
            }
            Err(error) => return Err(error),
        }
    }
}

/// Dónde está montado un volumen, si lo está.
pub async fn punto_de_montaje(conexion: &Connection, volumen: &OwnedObjectPath) -> Option<String> {
    let valor = propiedad(conexion, volumen, IFACE_ARCHIVOS, "MountPoints")
        .await
        .ok()?;
    let puntos = Vec::<Vec<u8>>::try_from(valor).ok()?;

    puntos
        .into_iter()
        .next()
        .map(|bytes| ruta_desde_bytes(&bytes))
}

/// `udisks2` devuelve las rutas como bytes terminados en cero, porque un nombre
/// de archivo en Linux no tiene por qué ser UTF-8.
pub fn ruta_desde_bytes(bytes: &[u8]) -> String {
    let sin_cero = bytes.split(|byte| *byte == 0).next().unwrap_or(bytes);
    String::from_utf8_lossy(sin_cero).into_owned()
}

/// Qué clase de error devolvió `udisks2`.
///
/// La distinción que importa es una sola: si tiene sentido volver a pedir la
/// frase. `Failed` es lo que devuelve `udisks2` cuando `cryptsetup` no pudo abrir
/// el volumen, y la causa abrumadoramente más común de eso es que la frase esté
/// mal — así que se reintenta. Que polkit haya negado la autorización, en cambio,
/// no se arregla escribiendo otra frase: el diálogo tiene que cerrarse y decir
/// qué pasó.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaseDeError {
    /// `cryptsetup` no abrió el volumen. Casi siempre, la frase está mal.
    FraseIncorrecta,
    /// La persona cerró el diálogo de polkit.
    Cancelado,
    /// polkit negó la autorización.
    NoAutorizado,
    /// El volumen ya estaba montado, que no es un fallo para quien quería llegar
    /// a los archivos.
    YaMontado,
    /// `udisks2` todavía no terminó de reconocer el volumen.
    TodaviaNoListo,
    /// Cualquier otra cosa: el disco ocupado, sin sistema de archivos conocido…
    Otro,
}

/// Un error de `udisks2`, ya clasificado.
#[derive(Debug, Clone)]
pub struct ErrorDeUdisks {
    pub mensaje: String,
    pub clase: ClaseDeError,
}

impl ErrorDeUdisks {
    /// Un error del que no se vuelve.
    pub fn definitivo(mensaje: String) -> Self {
        Self {
            mensaje,
            clase: ClaseDeError::Otro,
        }
    }

    /// Si vale la pena volver a pedir la frase.
    pub fn reintentable(&self) -> bool {
        self.clase == ClaseDeError::FraseIncorrecta
    }

    fn de_zbus(error: zbus::Error) -> Self {
        let (nombre, mensaje) = match &error {
            zbus::Error::MethodError(nombre, detalle, _) => (
                nombre.as_str().to_string(),
                detalle.clone().unwrap_or_default(),
            ),
            otro => (String::new(), otro.to_string()),
        };

        Self::clasificar(&nombre, &mensaje)
    }

    /// Qué significa cada error de `udisks2` para el diálogo.
    pub fn clasificar(nombre: &str, mensaje: &str) -> Self {
        let corto = nombre.rsplit('.').next().unwrap_or(nombre);
        let mensaje = if mensaje.trim().is_empty() {
            nombre.to_string()
        } else {
            mensaje.to_string()
        };

        let clase = match corto {
            "Failed" => ClaseDeError::FraseIncorrecta,
            "NotAuthorizedDismissed" => ClaseDeError::Cancelado,
            "NotAuthorized" | "NotAuthorizedCanObtain" => ClaseDeError::NoAutorizado,
            "AlreadyMounted" => ClaseDeError::YaMontado,
            // El volumen existe pero todavía no tiene la interfaz de sistema de
            // archivos: `udisks2` no terminó de sondearlo. Ver
            // `montar_cuando_este_listo`.
            "UnknownInterface" | "UnknownMethod" | "UnknownObject" => ClaseDeError::TodaviaNoListo,
            _ => ClaseDeError::Otro,
        };

        Self { mensaje, clase }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solo_las_rutas_bajo_dev_son_dispositivos() {
        assert!(ruta_de_dispositivo_valida("/dev/sda3"));
        assert!(ruta_de_dispositivo_valida("/dev/mapper/algo"));

        assert!(!ruta_de_dispositivo_valida("/home/pato/disco.img"));
        assert!(!ruta_de_dispositivo_valida("/dev/"));
        assert!(!ruta_de_dispositivo_valida("sda3"));
        assert!(!ruta_de_dispositivo_valida(""));
    }

    #[test]
    fn una_ruta_con_dos_puntos_no_pasa() {
        // No es que `udisks2` la fuera a aceptar; es que el error que devuelve por
        // una ruta así es peor de leer que el que se da acá.
        assert!(!ruta_de_dispositivo_valida("/dev/../etc/shadow"));
    }

    #[test]
    fn una_frase_equivocada_se_puede_volver_a_escribir() {
        let error = ErrorDeUdisks::clasificar(
            "org.freedesktop.UDisks2.Error.Failed",
            "Error unlocking /dev/sda3: Failed to activate device: Operation not permitted",
        );

        assert!(error.reintentable());
        assert_eq!(error.clase, ClaseDeError::FraseIncorrecta);
    }

    #[test]
    fn una_autorizacion_negada_cierra_el_dialogo() {
        // Volver a pedir la frase acá sería pedirle a la persona que resuelva
        // escribiendo algo un problema que no se resuelve escribiendo nada.
        let error = ErrorDeUdisks::clasificar(
            "org.freedesktop.UDisks2.Error.NotAuthorizedDismissed",
            "Not authorized to perform operation",
        );

        assert!(!error.reintentable());
        assert_eq!(error.clase, ClaseDeError::Cancelado);
    }

    #[test]
    fn no_estar_autorizado_no_es_haber_cancelado() {
        let error = ErrorDeUdisks::clasificar(
            "org.freedesktop.UDisks2.Error.NotAuthorized",
            "Not authorized",
        );

        assert!(!error.reintentable());
        assert_eq!(error.clase, ClaseDeError::NoAutorizado);
    }

    #[test]
    fn ya_estar_montado_se_reconoce_para_poder_no_tratarlo_como_fallo() {
        let error = ErrorDeUdisks::clasificar(
            "org.freedesktop.UDisks2.Error.AlreadyMounted",
            "Device is already mounted",
        );

        assert_eq!(error.clase, ClaseDeError::YaMontado);
        assert!(!error.reintentable());
    }

    #[test]
    fn preguntar_demasiado_pronto_no_es_que_el_disco_no_sirva() {
        // Sin esto, montar un volumen recién abierto falla con «no existe ese
        // método» y parece que no tiene sistema de archivos.
        let error = ErrorDeUdisks::clasificar(
            "org.freedesktop.DBus.Error.UnknownMethod",
            "No such interface 'org.freedesktop.UDisks2.Filesystem'",
        );

        assert_eq!(error.clase, ClaseDeError::TodaviaNoListo);
        assert!(!error.reintentable());
    }

    #[test]
    fn un_error_desconocido_no_se_reintenta() {
        let error = ErrorDeUdisks::clasificar("org.freedesktop.UDisks2.Error.DeviceBusy", "busy");

        assert!(!error.reintentable());
        assert_eq!(error.clase, ClaseDeError::Otro);
    }

    #[test]
    fn un_error_sin_detalle_se_queda_con_el_nombre() {
        // Un mensaje vacío en el diálogo no dice nada; el nombre del error al
        // menos se puede buscar.
        let error = ErrorDeUdisks::clasificar("org.freedesktop.UDisks2.Error.DeviceBusy", "   ");

        assert_eq!(error.mensaje, "org.freedesktop.UDisks2.Error.DeviceBusy");
    }

    #[test]
    fn las_rutas_llegan_terminadas_en_cero() {
        assert_eq!(
            ruta_desde_bytes(b"/run/media/pato/Datos\0"),
            "/run/media/pato/Datos"
        );
        assert_eq!(ruta_desde_bytes(b"/mnt/disco"), "/mnt/disco");
    }

    #[test]
    fn un_volumen_sin_etiqueta_se_nombra_por_su_dispositivo() {
        let bloque = Bloque {
            objeto: OwnedObjectPath::try_from("/org/freedesktop/UDisks2/block_devices/sda3")
                .unwrap(),
            uuid: "1234".to_string(),
            etiqueta: "   ".to_string(),
        };

        assert_eq!(bloque.nombre_visible("/dev/sda3"), "/dev/sda3");
    }

    #[test]
    fn con_etiqueta_se_nombra_por_ella() {
        let bloque = Bloque {
            objeto: OwnedObjectPath::try_from("/org/freedesktop/UDisks2/block_devices/sda3")
                .unwrap(),
            uuid: "1234".to_string(),
            etiqueta: "Respaldos".to_string(),
        };

        assert_eq!(bloque.nombre_visible("/dev/sda3"), "Respaldos");
    }
}
