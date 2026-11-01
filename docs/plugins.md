# Plugins

## Qué es un plugin

Un plugin es una carpeta con un `plugin.toml`. Puede aportar un worker, un overlay estático o ambos. Se cargan una vez al arrancar `streams-tools`; un manifiesto o worker defectuoso no impide usar los demás. Los overlays quedan en `http://127.0.0.1:7777/p/<name>/`.

El directorio se elige con `STREAMS_PLUGINS_DIR` y por defecto es `./plugins`. Los datos persistentes del worker van a `STREAMS_DATA_DIR` (`./data` por defecto) y los estilos del usuario a `STREAMS_STYLES_DIR` (`./styles`). No hay recarga en caliente: reinicia el binario después de cambiar manifiesto o configuración.

## Estructura

```text
mi-plugin/
├── plugin.toml
├── worker/             # opcional: fuente del worker
├── bin/                # ejecutable que usará el manifiesto
└── overlay/            # opcional: HTML, CSS y otros estáticos
    └── index.html
```

`worker/` y `overlay/` son independientes. Un plugin puede limitarse a escuchar eventos o limitarse a mostrar una visual.

## Manifiesto

```toml
name = "mi-plugin"              # obligatorio: ^[a-z0-9-]+$ y nombre exacto de la carpeta
version = "0.1.0"             # string obligatorio, no vacío
description = "Mi visual"     # opcional, no vacío tras trim

[permissions]                  # obligatorio
listens = ["chat"]            # obligatorio; "*" solo y único significa todos
emits = ["mi_evento"]         # obligatorio; no admite "*"

[worker]                       # opcional
command = ["bin/mi-worker"]   # obligatorio si existe [worker]
command_windows = ["bin/mi-worker.exe"]
command_macos = ["bin/mi-worker"]
command_linux = ["bin/mi-worker"]

[config]                       # opcional, tabla libre; {} si falta
poll_seconds = 5
theme = "dark"
```

Los arrays no admiten strings vacíos ni duplicados. Selecciona el override del sistema actual y, si no existe, usa `command`. El primer elemento es el ejecutable y el resto son argumentos. Una ruta con separadores o que empieza por `.` se resuelve desde la raíz del plugin; en Windows se añade `.exe` si no tiene extensión. Un nombre como `node` se busca en `PATH`.

No existe permiso implícito para eventos núcleo (`follow`, `sub`, `gift_sub`, `raid`, `donation`, `redemption`, `chat`): hay que declararlos literalmente y sus datos deben respetar su esquema. `config` se entrega como objeto JSON al worker y al overlay.

## Protocolo worker

El worker se ejecuta con la raíz del plugin como directorio actual. Recibe por stdin una primera línea JSONL:

```json
{"type":"init","config":{"theme":"dark"},"data_dir":"/ruta/absoluta/data/mi-plugin"}
```

Después recibe los `Event` núcleo completos permitidos por `listens`. Por stdout devuelve una línea por evento: `{"type":"mi_evento","data":{...},"user":{...}}`; `user` es opcional. Si falta, núcleo usa el nombre del plugin como `id` y `name`, sin roles. Núcleo también completa `id`, `platform` y `ts`.

Reserva stdout exclusivamente para JSONL y escribe logs en stderr; se reenvían con prefijo `[plugin:<name>]`. JSON inválido, payloads núcleo inválidos y tipos no declarados se descartan sin matar el worker. Un fallo de escritura o la salida del proceso sí reinicia esa ejecución.

El backoff es 1/2/4/… segundos hasta 30. La sexta caída dentro de una ventana de 60 segundos deshabilita ese worker. El protocolo admite cualquier lenguaje, pero se recomienda Rust u otro binario nativo para minimizar RAM.

## Overlay

Sirve `overlay/` bajo `/p/<name>/`, con `index.html` automático para directorios. Usa `./config.json` para la configuración y `/streams.js` (ruta absoluta) para recibir eventos con `streams.on("mi_evento", callback)`. La carpeta es opcional y no necesita declararse en el manifiesto.

## Estilos

Enlaza primero el CSS del plugin y después `./custom.css`. Ese endpoint lee `styles/<name>.css` en cada petición; si no existe devuelve 200 vacío, así que puedes crearlo sin tocar el plugin y recargar el Browser Source.

Usa un contenedor `.st-<plugin>`, clases BEM `.st-<plugin>__<elemento>` y variables `--st-<plugin>-*` para colores, fuentes, tamaños, radios y duraciones. Evita estilos inline en JavaScript salvo variables CSS dinámicas. Ejemplo de `styles/echo.css`:

```css
.st-echo {
  --st-echo-accent: #ff7a90;
  --st-echo-font: Georgia, serif;
}
```

## Probar echo

Compila el worker y copia el binario a la ubicación del manifiesto:

```sh
cargo build --release -p echo-worker
mkdir -p examples/plugins/echo/bin
cp target/release/echo-worker examples/plugins/echo/bin/
```

En Windows copia `target/release/echo-worker.exe` como `examples/plugins/echo/bin/echo-worker.exe`. Después arranca:

```sh
STREAMS_PLUGINS_DIR=examples/plugins cargo run -p streams-tools-cli
```

Abre `http://127.0.0.1:7777/p/echo/`. Publica un chat completo:

```sh
curl -i http://127.0.0.1:7777/api/events \
  -H 'content-type: application/json' \
  -d '{"id":"chat-1","platform":"twitch","type":"chat","data":{"message":"!ping"},"user":{"id":"1","name":"ana","roles":[]},"ts":"2026-01-01T00:00:00Z"}'
```

El overlay recibe `pong` y muestra `pong`. También puedes copiar `examples/plugins/echo` a un directorio temporal y señalar su padre con `STREAMS_PLUGINS_DIR`.

## Publicar como submódulo

Publica el plugin en su propio repositorio con `plugin.toml` en la raíz. Desde `streams-tools`, usa HTTPS:

```sh
git submodule add -b main <url> plugins/<name>
git commit -m "chore(submodules): añade <name>"
```

Publica o mergea primero los cambios del repositorio del plugin. Después actualiza únicamente el puntero en `streams-tools`, en un commit `chore` propio, siguiendo `docs/submodules.md`.

## Dos recetas

- Spotify: `listens = []`, `emits = ["now_playing"]`; el worker consulta la API a su ritmo y el overlay escucha ese tipo.
- Cámara PTZ: `listens = ["redemption", "chat"]`, `emits = []`; recibe solo esos eventos y mueve hardware sin emitir ni servir overlay.

## Diagnóstico

Un error de manifiesto identifica ruta, campo y motivo. stdout no JSON o un tipo no permitido aparece en el log y se descarta. Un ejecutable ausente provoca reinicios con backoff y la deshabilitación eventual de ese worker; el servidor y los demás plugins continúan.
