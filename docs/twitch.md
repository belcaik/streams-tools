# Conectar Twitch

El adaptador usa EventSub por WebSocket: no necesitas un servidor público, abrir puertos ni pagar un servicio. Chat también llega por EventSub.

## 1. Crear una aplicación gratuita

1. Entra en <https://dev.twitch.tv/console/apps> con la cuenta del canal.
2. Registra una aplicación. Elige **Public** como Client Type. Twitch puede exigir una OAuth Redirect URL; usa `http://localhost`, aunque streams-tools no la utilizará porque inicia sesión con Device Code.
3. Copia el **Client ID**. No generes ni configures un client secret.

## 2. Configurar el canal

Crea `config.toml` junto al binario:

```toml
[twitch]
client_id = "tu-client-id"
channel = "login_del_canal"
```

Por defecto se reciben follow, sub, gift_sub, raid, redemption, chat y cheer. Para limitarlo:

```toml
[twitch]
client_id = "tu-client-id"
channel = "login_del_canal"
events = ["follow", "chat", "redemption"]
```

Puedes cambiar la ruta con `STREAMS_CONFIG`. Si el archivo o la sección `[twitch]` no existen, streams-tools arranca como antes y Twitch queda desactivado.

## 3. Primer inicio

Arranca `streams-tools`. La terminal mostrará una URL de Twitch y un código. Abre la URL, introduce el código y autoriza con la misma cuenta del canal configurado. El token se guarda en `${STREAMS_DATA_DIR}/twitch/token.json` (`./data/twitch/token.json` por defecto) y se renueva automáticamente. Trata ese archivo como una credencial y no lo compartas.

Los permisos se solicitan solo para los eventos elegidos: followers para follow; subscriptions para subs y regalos; redemptions para puntos del canal; chat para mensajes; y bits para cheers. Raid no necesita un scope adicional.

Los cheers se publican como eventos `donation`, con el número de bits como monto y `currency = "bits"`.

Si Twitch se desconecta, el resto de streams-tools sigue activo y el adaptador reintenta con backoff. Los estados `connected`, `disconnected` y `auth_required` se publican como eventos `system` de plataforma Twitch.
