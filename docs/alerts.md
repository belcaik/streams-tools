# Packs de alertas

Un pack es una carpeta de alertas reutilizable. Cada alerta combina un manifiesto `alert.toml`, su CSS obligatorio y recursos opcionales. El host carga y valida todo al arrancar; una alerta defectuosa aparece en el diagnóstico sin ocultar las demás.

El directorio predeterminado es `./alerts` y se cambia con `STREAMS_ALERTS_DIR`. Nombres de packs y alertas: `^[a-z0-9][a-z0-9-]*$`.

```text
alerts/
└── neon/                         # pack
    └── big-donation/             # alerta
        ├── alert.toml            # obligatorio
        ├── style.css             # obligatorio
        ├── index.html            # opcional, HTML estático confiable
        ├── script.js             # opcional, script clásico confiable
        ├── audio/donation.ogg
        └── images/confetti.webp
```

## Esquema de `alert.toml`

Toda tabla rechaza campos desconocidos. `event` admite `follow`, `sub`, `gift_sub`, `raid`, `donation`, `redemption` y `chat`.

| Campo | Default | Validez |
|---|---|---|
| `event` | obligatorio | Uno de los siete eventos núcleo |
| `template` | obligatorio | Texto con placeholders permitidos |
| `priority` | `0` | Entero `i32`; gana el mayor |
| `duration_ms` | `5000` | `1..=300000` |
| `volume` | `1.0` | Finito, `0..=1` |
| `sound` | ausente | Asset relativo existente |
| `conditions.currency` | ausente | Sólo `donation`; no vacío, ASCII sin distinguir mayúsculas |
| `conditions.min_amount` | ausente | Sólo `donation`; finito y `>= 0` |
| `conditions.max_amount` | ausente | Sólo `donation`; finito, `>= 0` y `>= min_amount` |
| `conditions.min_months` | ausente | Sólo `sub`; `> 0` |
| `conditions.min_count` | ausente | Sólo `gift_sub`; `> 0` |
| `conditions.min_viewers` | ausente | Sólo `raid`; `> 0` |
| `conditions.reward_title` | ausente | Sólo `redemption`; coincidencia exacta |
| `conditions.platforms` | ausente | Lista no vacía ni repetida de `twitch`, `kick`, `youtube` |
| `variations[].template` | ausente | Reemplaza el template base |
| `variations[].image` | ausente | Asset relativo existente |
| `variations[].sound` | ausente | Reemplaza el sonido base |
| `variations[].weight` | `1` | Entero `1..=10000` |

```toml
event = "donation"                 # obligatorio; uno de los 7 eventos núcleo
template = "{user}: {amount} {currency} — {message}" # obligatorio
priority = 10                       # opcional, default 0; i32
duration_ms = 6000                  # opcional, default 5000; 1..=300000
volume = 0.8                        # opcional, default 1.0; finito, 0..=1
sound = "audio/donation.ogg"        # opcional; ruta relativa portable

[conditions]                        # tabla opcional
currency = "USD"                   # sólo donation; comparación ASCII case-insensitive
min_amount = 5.0                    # sólo donation; finito y >= 0
max_amount = 100.0                  # sólo donation; finito, >= 0 y >= min_amount
min_months = 3                      # sólo sub
min_count = 5                       # sólo gift_sub
min_viewers = 10                    # sólo raid
reward_title = "Hydrate"            # sólo redemption; coincidencia exacta
platforms = ["twitch", "youtube"] # opcional para todos; twitch|kick|youtube, no duplicados

[[variations]]
template = "¡{user} aportó {amount} {currency}!" # opcional; reemplaza template base
image = "images/confetti.webp"      # opcional
sound = "audio/big.ogg"             # opcional; reemplaza sound base
weight = 3                          # opcional, default 1; 1..=10000
```

El ejemplo reúne todos los campos: en un manifiesto real, cada condición debe corresponder al `event` elegido.

## Placeholders

Sólo se aceptan estos nombres; llaves desbalanceadas o placeholders desconocidos invalidan el manifiesto. Un campo ausente se sustituye por texto vacío y el resultado siempre se inserta como texto plano.

| Placeholder | Origen |
|---|---|
| `{user}` | `event.user.name`, en todos |
| `{amount}` | `donation.data.amount` |
| `{currency}` | `donation.data.currency` |
| `{months}` | `sub.data.months` |
| `{count}` | `gift_sub.data.count` |
| `{viewers}` | `raid.data.viewers` |
| `{reward}` | `redemption.data.reward_title` |
| `{message}` | `donation.data.message`, `redemption.data.input` o `chat.data.message` |
| `{tier}` | `sub.data.tier` o `gift_sub.data.tier` |

## Variantes y selección

Cada variación debe cambiar al menos `template`, `image` o `sound`; cambiar sólo `weight` no es válido. La elección usa acumulado entero de pesos. Una variación hereda template y sonido base cuando no los reemplaza; sin variaciones se usa la base.

Entre alertas compatibles se elige mayor `priority`, después mayor especificidad (un punto por cada condición presente), orden de `config.packs` —o del índice si está vacío— y nombre ascendente. `chat` es válido en un pack, pero el plugin lo desactiva por defecto.

## Rutas y assets

Las rutas usan `/`, no pueden ser absolutas ni vacías y no admiten `\`, componentes vacíos, `.` ni `..`. Cada componente sólo admite ASCII alfanumérico, `.`, `_` y `-`. `sound`, `image` y los sonidos de variantes deben existir y ser archivos regulares; `style.css` también es obligatorio.

El host canonicaliza cada archivo y exige que permanezca bajo la raíz canónica de la alerta. Al servirlo repite la validación y canonicalización, exige `is_file()` y devuelve 404 ante cualquier fallo. Así bloquea traversal y symlinks que escapen del pack. No se publican rutas absolutas en el índice.

Las URLs de recursos son `/alerts/<pack>/<alerta>/<asset>`. Tipos MIME reconocidos: HTML, CSS, JS, JSON, PNG, JPG/JPEG, GIF, WebP, SVG, WAV, MP3 y OGG; los demás usan `application/octet-stream`.

## Instalación y reinicio

Copia un pack completo:

```sh
cp -R /ruta/al/pack alerts/neon
```

O instala su repositorio como submódulo desde la raíz de `streams-tools`:

```sh
git submodule add -b main <url> alerts/<pack>
```

Publica primero el repositorio del pack y actualiza el puntero en un commit `chore` aislado. No hay recarga en caliente: **reinicia `streams-tools` después de añadir, quitar o modificar packs, manifiestos o assets**.

Con el plugin `alerts` instalado están disponibles:

- Overlay: `http://127.0.0.1:7777/p/alerts/`.
- Página de prueba: `http://127.0.0.1:7777/p/alerts/test.html`.
- Catálogo y errores: `http://127.0.0.1:7777/alerts/index.json`.

## Configuración del plugin

```toml
[config]
packs = []
volume = 1.0
gap_ms = 250
max_queue = 20
events = ["follow", "sub", "gift_sub", "raid", "donation", "redemption"]
chat_alerts = false
test_mode = false
```

| Clave | Default | Regla |
|---|---|---|
| `packs` | `[]` | Todos los externos en orden del índice; también puede incluir `default` |
| `volume` | `1.0` | Multiplica el volumen de alerta, saturado a `0..=1` |
| `gap_ms` | `250` | Pausa entre alertas, `0..=60000` |
| `max_queue` | `20` | Cola FIFO, `1..=100`; al llenarse descarta el evento nuevo |
| `events` | seis sin `chat` | Tipos núcleo únicos habilitados |
| `chat_alerts` | `false` | Chat exige además `chat` en `events` |
| `test_mode` | `false` | Encola ejemplos deterministas tras cargar el catálogo |

El pack incorporado `default` se activa si no hay alertas externas válidas o si `packs` lo nombra explícitamente. Además, cuando ningún pack activo tiene una alerta para un evento, se usa la alerta equivalente de `default`: un pack que solo personaliza donaciones no deja sin alerta a follows o subs.

## Personalización CSS

Crea `styles/alerts.css` (o `<STREAMS_STYLES_DIR>/alerts.css`). El overlay enlaza el CSS de cada alerta antes de `./custom.css`, por lo que este archivo queda último y puede recargarse sin reiniciar el host.

La raíz es `.st-alerts`; los elementos usan BEM: `.st-alerts__alert`, `__image`, `__content`, `__title` y `__message`. La alerta lleva `data-pack`, `data-alert`, el modificador `--<event>` y estados `--entering`, `--visible`, `--leaving`.

Variables disponibles: `--st-alerts-font-family`, `--st-alerts-width`, `--st-alerts-color`, `--st-alerts-background`, `--st-alerts-accent`, `--st-alerts-padding`, `--st-alerts-gap`, `--st-alerts-radius`, `--st-alerts-shadow`, `--st-alerts-title-size`, `--st-alerts-message-size`, `--st-alerts-image-size`, `--st-alerts-enter-duration`, `--st-alerts-exit-duration`, `--st-alerts-enter-transform` y `--st-alerts-exit-transform`.

```css
.st-alerts {
  --st-alerts-accent: #8af;
  --st-alerts-width: 36rem;
}

.st-alerts__alert[data-pack="neon"][data-alert="big-donation"] {
  --st-alerts-background: #102030;
  --st-alerts-enter-transform: translateY(2rem);
}
```

## Probar los siete eventos

Cada petición usa el esquema núcleo completo. Cambia `id` si repites una prueba y el adaptador aplica deduplicación.

```sh
curl -i http://127.0.0.1:7777/api/events -H 'content-type: application/json' -d '{"id":"follow-1","platform":"twitch","type":"follow","data":{},"user":{"id":"1","name":"ana","roles":[]},"ts":"2026-01-01T00:00:00Z"}'
curl -i http://127.0.0.1:7777/api/events -H 'content-type: application/json' -d '{"id":"sub-1","platform":"twitch","type":"sub","data":{"tier":"1000","months":3},"user":{"id":"2","name":"bea","roles":["subscriber"]},"ts":"2026-01-01T00:01:00Z"}'
curl -i http://127.0.0.1:7777/api/events -H 'content-type: application/json' -d '{"id":"gift-1","platform":"kick","type":"gift_sub","data":{"tier":"gold","count":5},"user":{"id":"3","name":"carla","roles":[]},"ts":"2026-01-01T00:02:00Z"}'
curl -i http://127.0.0.1:7777/api/events -H 'content-type: application/json' -d '{"id":"raid-1","platform":"twitch","type":"raid","data":{"viewers":42},"user":{"id":"4","name":"diego","roles":[]},"ts":"2026-01-01T00:03:00Z"}'
curl -i http://127.0.0.1:7777/api/events -H 'content-type: application/json' -d '{"id":"redemption-1","platform":"twitch","type":"redemption","data":{"reward_id":"reward-7","reward_title":"Hydrate","cost":500,"input":"hola"},"user":{"id":"6","name":"fede","roles":[]},"ts":"2026-01-01T00:05:00Z"}'
curl -i http://127.0.0.1:7777/api/events -H 'content-type: application/json' -d '{"id":"chat-1","platform":"kick","type":"chat","data":{"message":"buenas"},"user":{"id":"7","name":"gabi","roles":["mod"]},"ts":"2026-01-01T00:06:00Z"}'
```

Donación de bits en Twitch:

```sh
curl -i http://127.0.0.1:7777/api/events -H 'content-type: application/json' -d '{"id":"donation-bits","platform":"twitch","type":"donation","data":{"amount":500,"currency":"bits","message":"gran stream"},"user":{"id":"5","name":"eva","roles":[]},"ts":"2026-01-01T00:04:00Z"}'
```

Super Chat de YouTube y donación equivalente de Kick:

```sh
curl -i http://127.0.0.1:7777/api/events -H 'content-type: application/json' -d '{"id":"super-chat-1","platform":"youtube","type":"donation","data":{"amount":5,"currency":"USD","message":"gracias"},"user":{"id":"8","name":"ivan","roles":[]},"ts":"2026-01-01T00:07:00Z"}'
curl -i http://127.0.0.1:7777/api/events -H 'content-type: application/json' -d '{"id":"donation-kick","platform":"kick","type":"donation","data":{"amount":5,"currency":"USD","message":"gracias"},"user":{"id":"9","name":"julia","roles":[]},"ts":"2026-01-01T00:08:00Z"}'
```

## Diagnóstico y seguridad

`/alerts/index.json` siempre responde 200 con `{packs, errors}`. Cada error contiene `pack`, `file`, `field` y `reason`; `alert` se omite para errores de carpeta de pack. Los paths de `file` son relativos, como `neon/big-donation/alert.toml`; el log CLI usa la ruta absoluta y prefijo `[alert]`.

Una alerta rota no afecta a las demás, y un pack válido se conserva aunque tenga cero alertas válidas. Se ignoran archivos sueltos y carpetas sin `alert.toml`; una raíz ausente produce un catálogo vacío. Un fallo de lectura de toda la raíz sí impide arrancar.

El HTML y `script.js` del disco se consideran código confiable del pack. Los datos de eventos nunca se interpolan en HTML: se escriben con `textContent`, y la imagen sólo se asigna a `img[data-image]`. Los eventos `st-alerts:show` y `st-alerts:hide` limitan `detail` a `{pack, alert, eventType}`, sin texto de chat. El navegador puede exigir interacción antes de permitir audio; el plugin captura el rechazo de reproducción.
