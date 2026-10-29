# Submódulos

`streams-tools` puede agrupar otros repositorios como submódulos. `scripts/setup.sh`
configura git para que trabajar con ellos sea casi transparente.

## Clonar

```sh
git clone --recurse-submodules <url>
cd streams-tools
./scripts/setup.sh
```

¿Clonaste sin `--recurse-submodules`? Ejecuta:

```sh
git submodule update --init --recursive
```

## Qué configura `setup.sh`

| Opción | Efecto |
|---|---|
| `submodule.recurse=true` | `pull`, `checkout` y `switch` actualizan submódulos solos |
| `push.recurseSubmodules=check` | Impide pushear si un submódulo tiene commits sin subir |
| `diff.submodule=log` | `git diff` muestra los commits del submódulo, no solo el SHA |
| `status.submoduleSummary=true` | `git status` resume los cambios de submódulos |
| `fetch.recurseSubmodules=on-demand` | Trae commits de submódulos cuando hacen falta |

## Añadir un submódulo

```sh
git submodule add -b main <url> <ruta>
git commit -m "chore(submodules): añade <nombre>"
```

Usa URLs HTTPS para que cualquiera pueda clonar sin claves SSH.

## Actualizar un submódulo a su última versión

```sh
git submodule update --remote <ruta>
git add <ruta>
git commit -m "chore(submodules): actualiza <nombre> a <versión o sha>"
```

## Cambiar código dentro de un submódulo

1. Entra al submódulo, crea una rama y haz commits allí (con sus propias reglas).
2. Pushea el submódulo **primero** y abre su PR.
3. Cuando esté mergeada, actualiza el puntero en este repo en un commit `chore` aparte.

Los hooks bloquean un push si el puntero apunta a un commit que no existe en el remoto.

## Quitar un submódulo

```sh
git submodule deinit -f <ruta>
git rm -f <ruta>
rm -rf .git/modules/<ruta>
git commit -m "chore(submodules): elimina <nombre>"
```
