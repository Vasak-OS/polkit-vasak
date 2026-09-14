//! Abrir un disco cifrado: el diálogo, y lo que pasa detrás.
//!
//! ── Por qué la frase de un disco se pide acá ────────────────────────────────
//!
//! Este agente ya es la única aplicación de VasakOS cuyo trabajo es pedir una
//! contraseña. Un diálogo de contraseña dentro de cada aplicación multiplica los
//! lugares donde se escribe una, y cuantos más son, menos vale cada uno como
//! señal: si la contraseña se escribe en cualquier ventana, ninguna ventana es
//! sospechosa.
//!
//! El otro motivo es más concreto: así la frase no pasa por el gestor de
//! archivos. Quien la escribe es este proceso y quien la usa contra `udisks2`
//! también; el que llamó recibe el punto de montaje y nada más.
//!
//! ── Qué tiene que ver esto con polkit ───────────────────────────────────────
//!
//! Nada, y por eso el diálogo es otro. La frase de un disco no es la contraseña
//! de la cuenta, y presentarlas con el mismo texto —«Autenticación requerida»—
//! es lo que enseña a escribir la contraseña de la sesión donde no va. Comparten
//! la ventana y el proceso porque son el mismo lugar del escritorio, no porque
//! sean lo mismo.
//!
//! Abrir un disco *interno* sí requiere además autorización de polkit, así que
//! los dos diálogos pueden aparecer uno tras otro. Son dos preguntas distintas y
//! se hacen por separado, en ese orden.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::oneshot;
use tokio::sync::Mutex as MutexAsincrono;
use zbus::{interface, Connection, DBusError};
use zeroize::Zeroizing;
use zvariant::OwnedObjectPath;

use crate::llavero;
use crate::udisks::{self, Bloque, ClaseDeError, ErrorDeUdisks, EstadoDeCifrado};

/// El nombre y el objeto donde escucha este servicio.
pub const NOMBRE_DE_BUS: &str = "ar.net.vasak.os.DeviceUnlock";
pub const RUTA: &str = "/ar/net/vasak/os/DeviceUnlock";

/// Cuántas veces se puede volver a escribir la frase antes de darse por vencido.
///
/// El límite no es contra la fuerza bruta —quien tiene el disco puede probar
/// frases con `cryptsetup` todo lo que quiera, sin este diálogo de por medio—
/// sino contra el diálogo que no se cierra nunca: a la tercera, lo más probable
/// es que la frase no esté a mano, y dejar la ventana pidiéndola no ayuda.
const INTENTOS: usize = 3;

static CONTADOR: AtomicU64 = AtomicU64::new(1);

/// Lo que el diálogo contesta.
pub enum RespuestaDelDialogo {
    Frase {
        frase: Zeroizing<String>,
        recordar: bool,
    },
    Cancelado,
}

/// Los diálogos que están esperando una respuesta.
///
/// Es lo mismo que hace el agente de polkit con sus cookies, y por el mismo
/// motivo: la ventana contesta por un comando de Tauri, que no tiene forma de
/// alcanzar la tarea que está esperando si no es por acá.
#[derive(Clone, Default)]
pub struct DialogosPendientes {
    por_id: Arc<Mutex<HashMap<String, oneshot::Sender<RespuestaDelDialogo>>>>,
}

impl DialogosPendientes {
    fn registrar(&self, id: String, emisor: oneshot::Sender<RespuestaDelDialogo>) {
        if let Ok(mut pendientes) = self.por_id.lock() {
            pendientes.insert(id, emisor);
        }
    }

    fn responder(&self, id: &str, respuesta: RespuestaDelDialogo) -> Result<(), String> {
        let emisor = self
            .por_id
            .lock()
            .map_err(|error| error.to_string())?
            .remove(id)
            .ok_or_else(|| format!("no hay ningún desbloqueo esperando con el id {id}"))?;

        emisor
            .send(respuesta)
            .map_err(|_| "el desbloqueo ya no está esperando".to_string())
    }

    fn olvidar(&self, id: &str) {
        if let Ok(mut pendientes) = self.por_id.lock() {
            pendientes.remove(id);
        }
    }
}

/// Lo que puede salir mal, con un nombre que el que llamó pueda distinguir.
///
/// Distinguirlos importa: que la persona cierre el diálogo no es un fallo y no
/// merece un cartel de error, mientras que un disco ocupado sí. Un único error
/// con el texto adentro obligaría a quien llama a leer mensajes en inglés de
/// `udisks2` para decidir qué mostrar.
#[derive(Debug, DBusError)]
#[zbus(prefix = "ar.net.vasak.os.DeviceUnlock")]
pub enum ErrorDeDesbloqueo {
    /// Requerido por `zbus` para poder envolver sus propios errores.
    #[zbus(error)]
    ZBus(zbus::Error),
    /// La persona cerró el diálogo. No hay nada que avisar.
    Cancelado(String),
    /// polkit negó la autorización para abrir o montar.
    NoAutorizado(String),
    /// Se acabaron los intentos de escribir la frase.
    FraseIncorrecta(String),
    /// Cualquier otra cosa, con el detalle que haya dado `udisks2`.
    Fallo(String),
}

impl From<ErrorDeUdisks> for ErrorDeDesbloqueo {
    fn from(error: ErrorDeUdisks) -> Self {
        match error.clase {
            ClaseDeError::Cancelado => ErrorDeDesbloqueo::Cancelado(error.mensaje),
            ClaseDeError::NoAutorizado => ErrorDeDesbloqueo::NoAutorizado(error.mensaje),
            ClaseDeError::FraseIncorrecta => ErrorDeDesbloqueo::FraseIncorrecta(error.mensaje),
            _ => ErrorDeDesbloqueo::Fallo(error.mensaje),
        }
    }
}

/// El servicio que atiende los pedidos de desbloqueo.
pub struct DesbloqueoDeDispositivos {
    app: AppHandle,
    /// Para hablar con `udisks2`.
    sistema: Connection,
    /// Para hablar con el llavero.
    sesion: Connection,
    pendientes: DialogosPendientes,
    /// Un disco por vez.
    ///
    /// La ventana es una sola: dos pedidos simultáneos mostrarían dos diálogos
    /// encimados y la frase de uno podría terminar abriendo el otro.
    turno: Arc<MutexAsincrono<()>>,
}

#[interface(name = "ar.net.vasak.os.DeviceUnlock")]
impl DesbloqueoDeDispositivos {
    /// Abre el disco si hace falta, lo monta, y devuelve dónde quedó.
    ///
    /// Que el volumen no esté cifrado no es un error: se monta igual. La
    /// alternativa —contestar «esto no es un disco cifrado»— convierte en una
    /// trampa a un método que quien llama usa justamente cuando no está seguro.
    async fn unlock_and_mount(&self, device: &str) -> Result<String, ErrorDeDesbloqueo> {
        let _turno = self.turno.lock().await;

        let resultado = self.resolver(device).await;
        self.ocultar_ventana();

        if let Err(error) = &resultado {
            eprintln!("[vasak-polkit] desbloqueo de {device}: {error:?}");
        }

        resultado
    }
}

impl DesbloqueoDeDispositivos {
    async fn resolver(&self, dispositivo: &str) -> Result<String, ErrorDeDesbloqueo> {
        let bloque = udisks::describir(&self.sistema, dispositivo)
            .await
            .map_err(ErrorDeDesbloqueo::Fallo)?;
        let nombre = bloque.nombre_visible(dispositivo);

        let en_claro = match udisks::estado_de_cifrado(&self.sistema, &bloque.objeto).await {
            EstadoDeCifrado::Ninguno => bloque.objeto.clone(),
            EstadoDeCifrado::Abierto(volumen) => volumen,
            EstadoDeCifrado::Bloqueado => self.abrir(&bloque, &nombre).await?,
        };

        Ok(udisks::montar_cuando_este_listo(&self.sistema, &en_claro).await?)
    }

    /// Consigue la frase y abre el volumen.
    async fn abrir(
        &self,
        bloque: &Bloque,
        nombre: &str,
    ) -> Result<OwnedObjectPath, ErrorDeDesbloqueo> {
        if let Some(volumen) = self.abrir_con_lo_recordado(bloque).await? {
            return Ok(volumen);
        }

        let mut frase_incorrecta = false;

        for restantes in (0..INTENTOS).rev() {
            let (frase, recordar) = self.preguntar(nombre, frase_incorrecta).await?;

            match udisks::desbloquear(&self.sistema, &bloque.objeto, &frase).await {
                Ok(volumen) => {
                    if recordar {
                        self.recordar(bloque, nombre, &frase).await;
                    }
                    return Ok(volumen);
                }
                Err(error) if error.reintentable() && restantes > 0 => {
                    frase_incorrecta = true;
                }
                Err(error) => return Err(error.into()),
            }
        }

        // Inalcanzable: el último intento cae siempre en el brazo que devuelve el
        // error. Está para que el compilador no tenga que creerme.
        Err(ErrorDeDesbloqueo::FraseIncorrecta(
            "se agotaron los intentos".to_string(),
        ))
    }

    /// Prueba la frase guardada, si hay alguna.
    ///
    /// Una frase guardada que ya no abre el disco se borra: alguien la cambió con
    /// `cryptsetup`, y dejarla ahí sólo garantiza un intento fallido cada vez que
    /// se enchufe el disco.
    async fn abrir_con_lo_recordado(
        &self,
        bloque: &Bloque,
    ) -> Result<Option<OwnedObjectPath>, ErrorDeDesbloqueo> {
        let Some(frase) = llavero::frase_guardada(&self.sesion, &bloque.uuid).await else {
            return Ok(None);
        };

        match udisks::desbloquear(&self.sistema, &bloque.objeto, &frase).await {
            Ok(volumen) => Ok(Some(volumen)),
            Err(error) if error.reintentable() => {
                llavero::olvidar_frase(&self.sesion, &bloque.uuid).await;
                Ok(None)
            }
            Err(error) => Err(error.into()),
        }
    }

    /// Guarda la frase. Que no se pueda no hace fracasar el desbloqueo.
    async fn recordar(&self, bloque: &Bloque, nombre: &str, frase: &str) {
        if let Err(error) = llavero::recordar_frase(&self.sesion, &bloque.uuid, nombre, frase).await
        {
            // El disco ya está abierto; lo único que se perdió es el recuerdo.
            // Fallar acá sería deshacer un montaje que salió bien porque el
            // llavero está cerrado.
            eprintln!("[vasak-polkit] no se pudo recordar la frase: {error}");
        }
    }

    /// Muestra el diálogo y espera la respuesta.
    async fn preguntar(
        &self,
        nombre: &str,
        frase_incorrecta: bool,
    ) -> Result<(Zeroizing<String>, bool), ErrorDeDesbloqueo> {
        let id = format!("desbloqueo-{}", CONTADOR.fetch_add(1, Ordering::Relaxed));
        let (emisor, receptor) = oneshot::channel();
        self.pendientes.registrar(id.clone(), emisor);

        if let Some(ventana) = self.app.get_webview_window("main") {
            let _ = ventana.show();
            let _ = ventana.set_focus();
        }

        // El motivo va como bandera y no como texto: el mensaje lo arma la
        // ventana, que es la que tiene los idiomas.
        let _ = self.app.emit(
            "unlock-request",
            serde_json::json!({
                "id": id,
                "name": nombre,
                "wrongPassphrase": frase_incorrecta,
            }),
        );

        match receptor.await {
            Ok(RespuestaDelDialogo::Frase { frase, recordar }) => Ok((frase, recordar)),
            Ok(RespuestaDelDialogo::Cancelado) => Err(ErrorDeDesbloqueo::Cancelado(
                "se cerró el diálogo".to_string(),
            )),
            // El otro extremo se cayó: la ventana se cerró sin contestar.
            Err(_) => {
                self.pendientes.olvidar(&id);
                Err(ErrorDeDesbloqueo::Cancelado(
                    "el diálogo se cerró sin respuesta".to_string(),
                ))
            }
        }
    }

    fn ocultar_ventana(&self) {
        let _ = self.app.emit("unlock-done", serde_json::json!({}));
        if let Some(ventana) = self.app.get_webview_window("main") {
            let _ = ventana.hide();
        }
    }
}

/// La frase que la persona escribió.
#[tauri::command]
pub async fn enviar_frase(
    estado: State<'_, DialogosPendientes>,
    id: String,
    frase: String,
    recordar: bool,
) -> Result<(), String> {
    estado.responder(
        &id,
        RespuestaDelDialogo::Frase {
            frase: Zeroizing::new(frase),
            recordar,
        },
    )
}

/// La persona cerró el diálogo.
#[tauri::command]
pub async fn cancelar_desbloqueo(
    estado: State<'_, DialogosPendientes>,
    id: String,
) -> Result<(), String> {
    estado.responder(&id, RespuestaDelDialogo::Cancelado)
}

/// Publica el servicio en el bus de sesión.
///
/// Que falle no se lleva puesto al agente de polkit: son dos cosas separadas que
/// comparten proceso, y quedarse sin desbloqueo de discos es mejor que quedarse
/// sin poder autenticar nada.
pub async fn publicar(app: AppHandle, pendientes: DialogosPendientes) -> Result<(), String> {
    let sesion = Connection::session()
        .await
        .map_err(|error| format!("no hay bus de sesión: {error}"))?;
    let sistema = Connection::system()
        .await
        .map_err(|error| format!("no hay bus de sistema: {error}"))?;

    let servicio = DesbloqueoDeDispositivos {
        app,
        sistema,
        sesion: sesion.clone(),
        pendientes,
        turno: Arc::new(MutexAsincrono::new(())),
    };

    sesion
        .object_server()
        .at(RUTA, servicio)
        .await
        .map_err(|error| format!("no se pudo publicar el objeto: {error}"))?;

    sesion
        .request_name(NOMBRE_DE_BUS)
        .await
        .map_err(|error| format!("no se pudo tomar el nombre {NOMBRE_DE_BUS}: {error}"))?;

    eprintln!("[vasak-polkit] desbloqueo de discos publicado en {NOMBRE_DE_BUS}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn responder_un_dialogo_que_no_existe_no_entra_en_panico() {
        let pendientes = DialogosPendientes::default();

        let resultado = pendientes.responder("no-existe", RespuestaDelDialogo::Cancelado);

        assert!(resultado.is_err());
    }

    #[test]
    fn un_dialogo_registrado_recibe_la_respuesta() {
        let pendientes = DialogosPendientes::default();
        let (emisor, mut receptor) = oneshot::channel();
        pendientes.registrar("d-1".to_string(), emisor);

        pendientes
            .responder(
                "d-1",
                RespuestaDelDialogo::Frase {
                    frase: Zeroizing::new("abrite sésamo".to_string()),
                    recordar: true,
                },
            )
            .expect("tenía que llegar");

        match receptor.try_recv().expect("tenía que haber algo") {
            RespuestaDelDialogo::Frase { frase, recordar } => {
                assert_eq!(frase.as_str(), "abrite sésamo");
                assert!(recordar);
            }
            RespuestaDelDialogo::Cancelado => panic!("se esperaba una frase"),
        }
    }

    #[test]
    fn un_dialogo_se_contesta_una_sola_vez() {
        // Sin esto, dos clics seguidos en «Aceptar» mandarían dos frases y la
        // segunda quedaría dando vueltas sin nadie que la espere.
        let pendientes = DialogosPendientes::default();
        let (emisor, _receptor) = oneshot::channel();
        pendientes.registrar("d-1".to_string(), emisor);

        assert!(pendientes
            .responder("d-1", RespuestaDelDialogo::Cancelado)
            .is_ok());
        assert!(pendientes
            .responder("d-1", RespuestaDelDialogo::Cancelado)
            .is_err());
    }

    #[test]
    fn olvidar_un_dialogo_lo_saca_de_la_lista() {
        let pendientes = DialogosPendientes::default();
        let (emisor, _receptor) = oneshot::channel();
        pendientes.registrar("d-1".to_string(), emisor);

        pendientes.olvidar("d-1");

        assert!(pendientes
            .responder("d-1", RespuestaDelDialogo::Cancelado)
            .is_err());
    }

    #[test]
    fn cancelar_no_es_un_fallo_y_tiene_su_propio_nombre() {
        // Es lo que le permite al gestor de archivos no mostrar un cartel de
        // error cuando la persona simplemente cerró el diálogo.
        let cancelado: ErrorDeDesbloqueo = ErrorDeUdisks::clasificar(
            "org.freedesktop.UDisks2.Error.NotAuthorizedDismissed",
            "dismissed",
        )
        .into();

        assert!(matches!(cancelado, ErrorDeDesbloqueo::Cancelado(_)));
    }

    #[test]
    fn una_frase_equivocada_llega_como_tal_y_no_como_fallo_generico() {
        let error: ErrorDeDesbloqueo =
            ErrorDeUdisks::clasificar("org.freedesktop.UDisks2.Error.Failed", "no key available")
                .into();

        assert!(matches!(error, ErrorDeDesbloqueo::FraseIncorrecta(_)));
    }

    #[test]
    fn un_disco_ocupado_es_un_fallo_comun() {
        let error: ErrorDeDesbloqueo =
            ErrorDeUdisks::clasificar("org.freedesktop.UDisks2.Error.DeviceBusy", "busy").into();

        assert!(matches!(error, ErrorDeDesbloqueo::Fallo(_)));
    }
}
