# Instrucciones para agentes

Sigue [CONTRIBUTING.md](CONTRIBUTING.md) sin excepciones:

- Commits con Conventional Commits y atómicos: un cambio lógico por commit.
- PRs pequeñas: objetivo ≤ 400 líneas y ≤ 10 archivos. Si una tarea crece más, divídela en PRs encadenadas.
- Nunca uses `--no-verify` ni `STREAMS_SKIP_HOOKS=1` para esquivar los hooks.
- Submódulos: lee [docs/submodules.md](docs/submodules.md). No modifiques código dentro de un submódulo desde este repo; actualiza solo el puntero en un commit `chore` propio.
- Herramienta de orquestación de agentes: estado en `orchestration/` y `worktrees/`, ambos ignorados por git.
