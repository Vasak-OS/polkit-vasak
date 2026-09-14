# vasak-polkit-agent

Agente de autenticación de PolicyKit para VasakOS.

## ¿Qué es?

`vasak-polkit-agent` es un agente de PolicyKit que se registra en el bus
D-Bus del sistema para manejar solicitudes de autenticación de aplicaciones
como `pkexec`. Muestra una ventana minimalista para que el usuario ingrese
su contraseña y completa el flujo de autenticación.

## Arquitectura

```mermaid
sequenceDiagram
    participant pkexec
    participant polkitd
    participant D-Bus
    participant Agent as agente (zbus)
    participant Tauri
    participant Vue as diálogo Vue
    participant Helper as polkit-agent-helper-dbus (setuid root)
    participant PAM

    pkexec->>polkitd: solicita autenticación
    polkitd->>D-Bus: BeginAuthentication
    D-Bus->>Agent: BeginAuthentication(action, cookie, identities)
    Agent->>Tauri: emit polkit-request
    Tauri->>Vue: muestra diálogo
    Vue-->>Tauri: contraseña
    Tauri->>Agent: submit_password
    Agent->>Helper: spawn (uid/pid en argv; cookie+password por stdin)
    Helper->>PAM: authenticate(polkit-1) como el usuario de la identidad
    PAM-->>Helper: ok/error
    alt PAM ok
        Helper->>polkitd: AuthenticationAgentResponse3(cookie, identity, subject)
        polkitd-->>Helper: MethodReturn
        Helper-->>Agent: exit 0 (SUCCESS)
        Agent-->>polkitd: MethodReturn (session path)
        polkitd-->>pkexec: autorizado
    else PAM error / helper exit != 0
        Agent-->>Vue: polkit-result (error)
        Agent-->>polkitd: MethodReturn (fallo)
        polkitd-->>pkexec: denegado
    end
```

### Componentes

- **`vasak-polkit-agent`** — Binario principal (Tauri + zbus), corre **sin privilegios**.
  - Se registra como agente PolicyKit en la sesión del usuario.
  - Recibe `BeginAuthentication` vía D-Bus, muestra un diálogo de contraseña.
  - **No autentica él mismo**: entrega cookie y contraseña (por stdin) al helper
    setuid, que es el componente de confianza.
  - Bloquea `BeginAuthentication` hasta que el helper completa la llamada
    D-Bus (requisito de polkitd ≥ 127).

- **`polkit-agent-helper-dbus`** — Helper **setuid root** (componente de confianza).
  - polkitd solo acepta `AuthenticationAgentResponse3` desde uid 0, por eso es
    setuid (igual que el `polkit-agent-helper-1` estándar).
  - Lee cookie y contraseña **por stdin** (nunca argv, para no filtrarlas por `ps`).
  - **Autentica vía PAM (`polkit-1`) la identidad exacta que polkit pidió**
    (resuelta desde el uid), y solo entonces responde.
  - Abre un pidfd del proceso solicitante (`pidfd_open`) y lee `start-time` de
    `/proc/PID/stat`; envía el subject `unix-process` (pid + pidfd + start-time).

## Discos cifrados

La otra cosa que este proceso pregunta, y que con polkit no tiene nada que ver:
la frase de paso de un volumen LUKS.

### Por qué acá

Es la única aplicación de VasakOS cuyo trabajo ya es pedir una contraseña. Un
diálogo de contraseña dentro de cada aplicación multiplica los lugares donde se
escribe una, y cuantos más son, menos vale cada uno como señal: si la contraseña
se escribe en cualquier ventana, ninguna ventana es sospechosa.

El otro motivo es más concreto: así la frase no pasa por el gestor de archivos.
La escribe este proceso y la usa este proceso contra `udisks2`; el que llamó
recibe el punto de montaje y nada más.

### El diálogo es otro

Comparten la ventana y el proceso, no la pregunta. La frase de un disco no es la
contraseña de la cuenta, y presentarlas con el mismo texto es lo que enseña a
escribir la contraseña de la sesión donde no va: distinto título, distinto ícono,
y un test comprueba que los títulos no coincidan.

Abrir un disco **interno** sí necesita además autorización de polkit, así que los
dos diálogos pueden aparecer uno tras otro. Cuál se ve lo decide `dialogoVisible`
(`src/tools/dialogos.ts`): mientras polkit esté preguntando, el de la frase se
esconde, porque el pedido de polkit llega *adentro* del desbloqueo y lo deja
esperando.

### La interfaz

| | |
|---|---|
| Bus | de sesión |
| Nombre | `ar.net.vasak.os.DeviceUnlock` |
| Objeto | `/ar/net/vasak/os/DeviceUnlock` |
| Método | `UnlockAndMount(s dispositivo) → s punto_de_montaje` |

```bash
gdbus call --session \
  --dest ar.net.vasak.os.DeviceUnlock \
  --object-path /ar/net/vasak/os/DeviceUnlock \
  --method ar.net.vasak.os.DeviceUnlock.UnlockAndMount /dev/sda3
```

Que el volumen no esté cifrado no es un error: se monta igual. La alternativa
—contestar «esto no es un disco cifrado»— convierte en una trampa a un método que
se usa justamente cuando no se sabe.

Los errores tienen nombre propio para que quien llama pueda decidir qué mostrar
sin leer los mensajes en inglés de `udisks2`:

| Error | Qué pasó |
|---|---|
| `…DeviceUnlock.Cancelado` | Se cerró el diálogo. No hay nada que avisar. |
| `…DeviceUnlock.NoAutorizado` | polkit negó la autorización. |
| `…DeviceUnlock.FraseIncorrecta` | Se acabaron los tres intentos. |
| `…DeviceUnlock.Fallo` | Cualquier otra cosa, con el detalle de `udisks2`. |

### Recordar la frase

La casilla la guarda en `vasak-keyring`, por el Secret Service, indexada por el
UUID del volumen —no por `/dev/sdX`, que cambia entre enchufadas—. Un disco que
se usa todos los días pide la frase todos los días, y a eso se le encuentra la
vuelta de la peor manera: eligiendo una frase corta.

Todo lo del llavero falla en silencio. Si está cerrado, se pide la frase como si
nunca se hubiera guardado; y si guardarla no se puede, el disco queda montado
igual y lo único que se pierde es el recuerdo. Una frase guardada que deja de
abrir el disco —alguien la cambió con `cryptsetup`— se borra sola, para no gastar
un intento fallido en cada enchufada.

### Lo que esto no decide

Quién puede pedir un desbloqueo. El bus de sesión no distingue aplicaciones, así
que cualquier programa de la sesión puede llamar al método. No es una capacidad
nueva —`udisks2` ya está en el bus y polkit es quien autoriza—, pero con una
frase guardada el desbloqueo pasa a ser silencioso. Por eso el diálogo **siempre
nombra el disco**: es lo único que le permite a la persona reconocer un pedido
que no hizo. Si alguna vez hace falta más, el camino es comprobar el ejecutable
del llamante por `pidfd`, como hace `vasak-permissions`.

## Requisitos

- Rust 1.85+
- Node.js 20+ / Bun
- Tauri CLI 2.x
- D-Bus
- Polkit ≥ 127
- `udisks2`, para abrir y montar discos cifrados
- `vasak-keyring` (opcional), para recordar la frase de paso

## Compilar

```bash
bun install
cargo tauri build
```

Los binarios se generan en `src-tauri/target/release/`:
- `vasak-polkit-agent`
- `polkit-agent-helper-dbus`

## Instalación

```bash
sudo install -m 755 src-tauri/target/release/vasak-polkit-agent /usr/bin/
sudo install -m 755 src-tauri/target/release/polkit-agent-helper-dbus /usr/bin/
```

Configuración D-Bus necesaria en
`/usr/share/dbus-1/system.d/org.freedesktop.PolicyKit1.conf`:

```xml
<policy user="polkitd">
  <allow send_interface="org.freedesktop.PolicyKit1.AuthenticationAgent"/>
</policy>
```

## Desarrollo

```bash
bun run tauri dev
```

Esto inicia el agente y el frontend con hot-reload. Ejecutar `pkexec id`
en otra terminal para probar.
