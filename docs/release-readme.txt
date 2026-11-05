streams-tools 0.1.1
===================

1. Copia o renombra config.example.toml como config.toml.
2. Añade repositorios públicos a [plugins].install o usa:
     streams-tools plugin add https://github.com/owner/repo@v0.1.0
3. Ejecuta streams-tools (streams-tools.exe en Windows).

Comandos: plugin add, update, remove y list. Los plugins y sus datos se crean
en plugins/ y data/ junto al ejecutable. También puedes configurar
STREAMS_HOME. Las rutas relativas de overrides se resuelven contra el cwd.

ADVERTENCIA: un plugin puede ejecutar su worker y servir su overlay. Instala
sólo código de confianza. SHA-256 comprueba el asset publicado, no al autor.
Si una descarga automática falla, el servidor continúa con lo ya instalado.
