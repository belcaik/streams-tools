# Conectar Kick

El adaptador es self-hosted y no requiere servidor público ni servicios de pago. En v1 usa, en modo de solo lectura y sin autenticación, el WebSocket público de Pusher que usa kick.com. No usa los webhooks oficiales de Kick porque exigen una URL pública. Este transporte y sus payloads no son una API oficial y Kick puede cambiarlos.

## Configurar el canal

Se recomienda obtener los IDs manualmente. Abre `https://kick.com/api/v2/channels/<slug>` en el navegador, sustituye `<slug>` por el nombre de la URL del canal y copia `id` como `channel_id` y `chatroom.id` como `chatroom_id`:

```toml
[kick]
channel = "slug_del_canal"
channel_id = 123456
chatroom_id = 654321
```

Si falta alguno, streams-tools intenta `GET https://kick.com/api/v2/channels/<slug>`. Cloudflare suele devolver 403 a clientes que no son navegadores; en ese caso se publica `system/disconnected` con instrucciones para configurar ambos IDs y se reintenta sin detener el resto del programa.

Por defecto se reciben `chat`, `sub`, `gift_sub`, `raid` y `follow`. Para limitarlo:

```toml
[kick]
channel = "slug_del_canal"
channel_id = 123456
chatroom_id = 654321
events = ["chat", "sub", "gift_sub"]
```

Kick no ofrece en estos canales un equivalente a puntos del canal ni donaciones nativas; el adaptador no genera `redemption` ni `donation`.

## Pusher

Los valores actuales solo deben cambiarse si Kick cambia su cliente web:

```toml
[kick.pusher]
app_key = "32cbd69e4b950bf97679"
cluster = "us2"
```

La conexión usa `chatrooms.<chatroom_id>.v2` y `channel.<channel_id>`, responde ping/pong, reconecta con backoff y publica estados `connected`/`disconnected` como eventos `system` de plataforma Kick.

## Payloads observados

Todas las formas siguientes son **no oficiales, verificar con captura real** antes de cambiar el mapeo:

- `App\Events\ChatMessageEvent`: `id`, `content`, `created_at`, `sender.id`, `sender.username` y `sender.identity.badges[].type`. Solo conserva los badges `broadcaster`, `moderator`, `vip` y `subscriber`.
- `App\Events\SubscriptionEvent` y `App\Events\ChannelSubscriptionEvent`: `id`, `user_id`, `username`, `months`, `created_at`. Kick se representa como tier `"1"`; si falta `months`, usa 1.
- `App\Events\GiftedSubscriptionsEvent`: `id`, `user_id`, `gifter_username`, `gifted_usernames[]`, `created_at`. Produce un `gift_sub` con `count` igual al número de receptores. `LuckyUsersWhoGotGiftSubscriptionsEvent` se ignora para no contar el mismo regalo dos veces.
- `App\Events\StreamHostEvent`: `id`, `host_username`, `number_viewers`, `created_at`. Produce `raid`.
- `App\Events\FollowersUpdated`: `id`, `user_id`, `username`, `followed`, `created_at`. Solo produce `follow` cuando `followed = true` y existe `username`.

En todos los casos se usa `id` cuando existe; si falta, se genera `kick-<contador>`. Se usa `created_at` cuando existe; si falta, se usa el reloj actual en RFC 3339 UTC. Eventos desconocidos se ignoran sin romper la sesión.
