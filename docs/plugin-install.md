# Instalación y actualización de plugins

`streams-tools` puede instalar releases de repositorios públicos de GitHub sin necesitar Git ni Cargo. Sólo acepta `https://github.com/<owner>/<repo>`; añade `@v0.1.0` para fijar una versión exacta. Sin pin usa el release estable más reciente.

## Comandos

```sh
streams-tools plugin add https://github.com/belcaik/streams-plugin-alerts@v0.1.0
streams-tools plugin list
streams-tools plugin update alerts
streams-tools plugin update
streams-tools plugin remove alerts
```

`add` falla si el nombre ya está ocupado. `update` sin nombre intenta todos los administrados y devuelve error si cualquiera falla, sin omitir los demás. Una fuente fijada se revalida, pero no salta a otra versión. `remove` sólo borra plugins con metadata administrada; protege carpetas copiadas o submódulos.

`list` no usa red. Sus estados son:

- `managed`: plugin y metadata válidos;
- `unmanaged`: carpeta válida que el instalador no administra;
- `broken`: metadata corrupta o instalación incompleta.

La metadata vive en `plugins/.streams-tools/installed/<nombre>.toml`. No contiene URLs temporales ni timestamps. Si está corrupta, corrígela manualmente antes de actualizar o eliminar.

## Instalación al arrancar

Declara una lista deseada en `config.toml`:

```toml
[plugins]
install = [
  "https://github.com/belcaik/streams-plugin-tts",
  "https://github.com/belcaik/streams-plugin-alerts@v0.1.0",
]
```

Al arrancar se normalizan y deduplican las fuentes, se instalan sólo las ausentes y nunca se actualizan automáticamente. Un fallo de red, API, checksum o paquete se diagnostica y el servidor continúa con los plugins disponibles. Si eliminas uno que sigue en `install`, reaparecerá en el próximo arranque.

## Configuración persistente

Los defaults pertenecen al `plugin.toml` del paquete. Guarda cambios locales en el archivo principal:

```toml
[plugins.config.tts]
volume = 0.8
show_text = false

[plugins.config.tts.engine]
kind = "espeak"

[plugins.config.interactions]
rules_file = "mis-reglas.toml"

[plugins.config.alerts]
volume = 0.7

[plugins.config.now-playing]
show_album = true
```

Las tablas se mezclan recursivamente; arrays y escalares reemplazan el valor completo. Toda clave debe existir en el manifiesto y conservar su categoría exacta. La configuración local sobrevive updates y llega igual al worker y a `/p/<nombre>/config.json`. Configurar un plugin aún ausente produce un aviso, no un error fatal. Reinicia para aplicar cambios.

## Seguridad y límites

> ADVERTENCIA: instalar un repositorio permite que streams-tools ejecute su worker y sirva su overlay; instala sólo código de confianza.

`SHA256SUMS` detecta corrupción o alteración respecto del asset publicado, pero release y checksum tienen la misma autoridad: no demuestra que el autor sea confiable. Durante la instalación no se ejecutan workers, scripts ni hooks.

Antes de publicar se valida el nombre, la versión, el target, el manifiesto y cada path del ZIP. Se rechazan rutas absolutas o con `..`, symlinks, tipos especiales y paquetes con más de 2.048 entradas, 100 MiB descargados, 64 MiB por archivo o 250 MiB descomprimidos. La publicación usa renames en el mismo filesystem y backups; ante un fallo intenta restaurar plugin y metadata anteriores.

## Rutas portables y desarrollo

En un ZIP release, `config.toml`, `plugins/`, `data/`, `alerts/` y `styles/` viven junto al ejecutable. En debug (`cargo run`) parten del cwd para conservar el flujo con submódulos. `STREAMS_HOME` cambia la raíz. Los overrides `STREAMS_CONFIG`, `STREAMS_PLUGINS_DIR`, `STREAMS_DATA_DIR`, `STREAMS_ALERTS_DIR` y `STREAMS_STYLES_DIR` tienen prioridad; una ruta relativa parte del cwd.

Los submódulos son sólo para desarrollo del host. No mezcles sus carpetas unmanaged con instalación administrada. Consulta [submodules.md](submodules.md) y usa `git submodule update --init --recursive` en un checkout de desarrollo.
