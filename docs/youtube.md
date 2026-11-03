# Conectar YouTube

El adaptador lee el chat en vivo con el endpoint interno Innertube que usa la web de YouTube. No necesita clave de API, OAuth ni cuota, y no usa Data API v3. Innertube y sus payloads no son una API oficial: YouTube puede cambiarlos y romper el adaptador.

## Configurar el directo

Configura un handle, con o sin `@`, o un id de canal que comience por `UC`:

```toml
[youtube]
channel = "@canal"
```

Con un handle, streams-tools consulta `https://www.youtube.com/@<handle>/live`; con un id usa `https://www.youtube.com/channel/<id>/live`. Primero analiza `ytInitialPlayerResponse`, porque relaciona `videoDetails.videoId` con `isLiveNow`/`isLive`. Si no obtiene el id por esa vía, usa el canonical `watch?v=` solo cuando el HTML también afirma que el video está en directo.

También puedes fijar el video explícitamente:

```toml
[youtube]
video_id = "dQw4w9WgXcQ"
```

Si aparecen `channel` y `video_id`, `video_id` tiene prioridad y evita la consulta de resolución del canal. Debe existir al menos uno. Si el canal no está en directo se publica `system/offline` y se comprueba otra vez cada 60 segundos, sin marcar desconexión ni aplicar backoff exponencial.

Por defecto se reciben `chat`, `donation`, `sub` y `gift_sub`. Para limitarlo:

```toml
[youtube]
channel = "canal"
events = ["chat", "donation"]
```

YouTube no tiene un equivalente a los puntos de canal de Twitch, por lo que este adaptador no genera eventos `redemption`.

## Sesión de chat

El adaptador abre `live_chat`, extrae `ytInitialData`, `INNERTUBE_API_KEY`, la versión del cliente web y la continuación inicial. La clave es interna de la página, no una credencial configurada por el usuario. Los mensajes incluidos en la página inicial se consideran historial: sus ids se recuerdan, pero no se publican, para evitar alertas antiguas.

Después consulta `youtubei/v1/live_chat/get_live_chat`, respeta la continuación y limita cada espera indicada por YouTube a entre 1 y 10 segundos. Al abrir el chat publica `system/connected`. Si desaparece el contenido o la siguiente continuación, interpreta que el directo terminó, publica `system/offline` y vuelve a resolver. Errores de red, HTTP o parseo publican `system/disconnected` y reintentan desde la resolución con backoff exponencial. El resto de streams-tools continúa activo.

streams-tools se identifica como `streams-tools/<versión>`; no imita un navegador. Si YouTube redirige a `consent.youtube.com`, responde con bloqueo o exige consentimiento, el adaptador publica una desconexión clara. Puedes configurar `video_id` para omitir la resolución del canal, pero streams-tools no gestiona cookies de consentimiento ni evade controles anti-bot.

## Payloads observados

Todas las formas siguientes son **no oficiales, verificar con captura real** antes de cambiar el mapeo:

- `addChatItemAction.item.liveChatTextMessageRenderer`: usa `id`, `timestampUsec`, `authorExternalChannelId`, `authorName.simpleText`, `authorBadges[]` y `message.runs[]`. Los runs concatenan `text`; para emoji usan el primer `shortcut` o, en su defecto, `emojiId`. `OWNER` produce `broadcaster`, `MODERATOR` produce `moderator`, badges con `customThumbnail` producen `subscriber` y `VERIFIED` se ignora. El orden fijo es `broadcaster`, `moderator`, `vip`, `subscriber`.
- `liveChatPaidMessageRenderer` y `liveChatPaidStickerRenderer`: producen `donation` a partir de `purchaseAmountText.simpleText`; el sticker usa mensaje vacío. La tabla de prefijos es `$`/`US$`→`USD`, `€`→`EUR`, `£`→`GBP`, `¥`→`JPY`, `₩`→`KRW`, `₹`→`INR`, `A$`→`AUD`, `CA$`→`CAD`, `MX$`→`MXN`, `NZ$`→`NZD` y `R$`→`BRL`. Un prefijo desconocido se conserva literalmente. Se aceptan coma o punto decimal y separadores de miles.
- `liveChatMembershipItemRenderer`: produce `sub`; usa `headerSubtext` o el nombre del nivel como `tier`, con fallback `member`. Busca el primer entero de `headerPrimaryText` para hitos como “Member for 12 months” o su traducción, con fallback de un mes.
- `liveChatSponsorshipsGiftPurchaseAnnouncementRenderer`: produce `gift_sub` con tier `member`; obtiene el primer entero de `header.liveChatSponsorshipsHeaderRenderer.primaryText`, sin depender del idioma.
- Cualquier otro renderer o acción, incluidos borrados, ticker y banners, se ignora sin terminar la sesión.

`timestampUsec` se convierte a RFC 3339 UTC con milisegundos. Los importes y textos localizados de YouTube pueden cambiar; actualiza fixtures a partir de capturas reales antes de ajustar el parser.
