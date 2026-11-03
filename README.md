# streams-tools

Herramientas para streaming. Pensado para ser sencillo y simple de usar.

## Empezar

```sh
git clone --recurse-submodules <url>
cd streams-tools
./scripts/setup.sh
```

`setup.sh` configura la plantilla de commit y el manejo de submódulos.
Solo requiere `git` y un shell POSIX.

## Comandos básicos

| Tarea | Comando |
|---|---|
| Actualizar todo (incluye submódulos) | `git pull` |
| Nueva rama | `git switch -c feat/descripcion` |
| Commit parcial y atómico | `git add -p && git commit` |
| Subir rama y abrir PR | `git push -u origin HEAD && gh pr create --fill` |
| Sincronizar submódulos | `git submodule update --init --recursive` |
| Actualizar un submódulo | `git submodule update --remote <ruta>` |

## Plataformas

- [Conectar un canal de Twitch](docs/twitch.md) sin servidor público mediante EventSub WebSocket.
- [Conectar un canal de Kick](docs/kick.md) sin servidor público mediante el WebSocket público de Pusher.
- [Conectar un canal de YouTube](docs/youtube.md) sin API key ni OAuth mediante el chat interno Innertube.

## Reglas

- **Conventional Commits** (`feat:`, `fix:`, `docs:`...).
- **Commits atómicos**: un cambio lógico por commit.
- **PRs pequeñas**: objetivo ≤ 400 líneas cambiadas. Es la regla más importante.

Detalles en [CONTRIBUTING.md](CONTRIBUTING.md) y [docs/submodules.md](docs/submodules.md).
