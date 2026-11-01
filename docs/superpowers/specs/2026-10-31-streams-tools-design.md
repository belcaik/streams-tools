# streams-tools — Diseño

Fecha: 2026-10-31 · Estado: aprobado

## Objetivo

Herramientas de streaming self-hosted, 100 % software libre, sencillas de usar y
extensibles con plugins plug and play. Sustituye servicios como StreamElements o
Streamlabs para: alertas personalizadas, TTS por puntos del canal e interacciones
de chat (comandos, combos de mods).

## Restricciones

- Solo software libre (MIT/Apache/GPL). Ningún servicio de pago ni API key de pago.
- Windows, Linux y macOS con el mismo binario fuente.
- Plataformas: Twitch, Kick y YouTube.
- Instalación: un binario `streams-tools` + `config.toml`. Sin runtimes obligatorios.
- Overlays como Browser Source de OBS (`http://localhost:7777/...`).

## Arquitectura

Monolito modular en Rust (workspace de Cargo). Las funcionalidades de usuario son
plugins que usan la misma API que cualquier plugin externo.

```
crates/
  core/        tipos de evento, bus (tokio::broadcast), permisos
  server/      axum: overlays estáticos, WebSocket del bus, streams.js
  plugins/     loader de manifiestos, supervisor de workers, recarga en caliente
  adapters/    twitch, kick, youtube → eventos comunes
  cli/         binario streams-tools (config, arranque)
plugins/       plugins como submódulos git (oficiales y de terceros)
alerts/        packs de alertas como submódulos git
```

Dependencias clave: `tokio`, `axum`, `serde`, `toml`, `rustls` (nunca OpenSSL),
`directories` para rutas de config/datos por sistema.

## Modelo de eventos

Sobre JSON común:

```json
{ "id": "uuid", "platform": "twitch|kick|youtube|plugin:<nombre>",
  "type": "follow", "user": { "id": "1", "name": "ana", "roles": ["mod"] },
  "data": {}, "ts": "2026-10-31T20:00:00Z" }
```

- Tipos núcleo con esquema fijo y versionado: `follow`, `sub`, `gift_sub`, `raid`,
  `donation`, `redemption`, `chat`.
- Tipos de plugin: nombre libre (`now_playing`, `camera_move`), permitidos solo si
  el manifiesto del plugin los declara en `emits`.

## Plugins

Un plugin es una carpeta en `plugins/<nombre>/`:

```
plugin.toml   nombre, versión, permisos (listens, emits, chat_write, network), config
overlay/      opcional: HTML/CSS/JS servido en /p/<nombre>/, usa streams.js
worker/       opcional: proceso en cualquier lenguaje, JSON por líneas en stdin/stdout
```

- `streams.js`: `on(type, fn)`, `emit(type, data)`, `config` del plugin.
- Worker: el núcleo lo arranca, lo reinicia con backoff si muere y le reenvía solo
  los eventos de `listens`. El manifiesto admite un comando por sistema operativo
  (`command.default`, `command.windows`, `command.macos`, `command.linux`).
- El núcleo descarta eventos y acciones que el manifiesto no declara.
- Recarga en caliente al cambiar un `plugin.toml` u `overlay/`.
- WASM queda fuera de v1.

Plugins oficiales (en este orden): `alerts`, `tts`, `interactions`.

## Alertas

Plugin `alerts`. Cada alerta de un pack es una carpeta con HTML/CSS/JS, sonido y un
TOML con condiciones (tipo de evento, rango de monto), duración y variaciones. Las
alertas se encolan y se muestran de una en una.

## TTS

Plugin `tts`. Worker que invoca Piper (local). El audio WAV se envía al overlay y
se reproduce allí, así OBS lo captura sin código de audio nativo.

| Plataforma | Disparador por defecto |
|---|---|
| Twitch | Canje de puntos del canal |
| YouTube | Super Chat o miembro |
| Kick | `!tts` para subs, con cooldown |

Cola única, filtro de palabras, longitud máxima y comando de saltar para mods.

## Errores

- Un adaptador caído se reconecta con backoff exponencial y emite un evento
  `system` visible en el log; el resto sigue funcionando.
- Un worker que falla más de 5 veces en 60 s queda deshabilitado con aviso.
- Un manifiesto inválido se rechaza con el campo y el motivo exactos.

## CI/CD

| Workflow | Disparador | Acción |
|---|---|---|
| `ci.yml` | push, PR | `fmt --check`, `clippy -D warnings`, `test` en ubuntu/windows/macos, submódulos recursivos |
| `pr-checks.yml` | PR | Título en Conventional Commits; aviso > 400 líneas, fallo > 800 |
| `release.yml` | tag `v*` | Binarios Linux x86_64/aarch64, Windows x86_64, macOS x86_64/aarch64 en GitHub Releases |

## Pruebas

- Unitarias por crate (eventos, permisos, parseo de manifiestos).
- Integración: plugin de ejemplo con worker falso que emite y recibe eventos.
- Adaptadores probados con payloads grabados de cada plataforma.

## Orden de implementación

Cada paso es una o más PRs ≤ 400 líneas:

1. Scaffold del workspace + workflows de CI.
2. `core`: eventos y bus.
3. `server`: HTTP/WS + `streams.js`.
4. `plugins`: loader, permisos, supervisor.
5. Adaptador Twitch; luego Kick y YouTube.
6. Plugins oficiales `alerts`, `tts`, `interactions`.
