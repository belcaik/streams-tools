# Guía de contribución

Tres reglas no negociables. CI valida las que pueden automatizarse.

## 1. Conventional Commits

Formato:

```
<tipo>(<scope opcional>)<!opcional>: <descripción>

[cuerpo opcional]

[footer opcional]
```

| Tipo | Uso |
|---|---|
| `feat` | Nueva funcionalidad |
| `fix` | Corrección de un bug |
| `docs` | Solo documentación |
| `style` | Formato, sin cambio de lógica |
| `refactor` | Cambio de código sin bug ni feature |
| `perf` | Mejora de rendimiento |
| `test` | Añadir o corregir pruebas |
| `build` | Sistema de build o dependencias |
| `ci` | Integración continua |
| `chore` | Mantenimiento (incluye actualizar punteros de submódulos) |
| `revert` | Revertir un commit previo |

Reglas:

- Descripción en **imperativo**, minúscula inicial, sin punto final, ≤ 72 caracteres.
- Scope en minúsculas: `feat(overlay): añade contador de viewers`.
- Cambios incompatibles: `feat!: ...` o footer `BREAKING CHANGE: ...`.
- El cuerpo explica el **porqué**, no el cómo.

Ejemplos:

```
feat(chat): muestra badges de suscriptor
fix(obs): reconecta el websocket tras un timeout
docs: documenta el flujo de submódulos
chore(deps): actualiza submódulo overlays a v1.2.0
```

## 2. Commits atómicos

Un commit = **un cambio lógico** que compila y pasa las pruebas por sí solo.

- Si el subject necesita "y", son dos commits.
- No mezcles refactor con feature, ni formato con lógica.
- Usa `git add -p` para separar cambios dentro de un mismo archivo.
- Actualizar el puntero de un submódulo es su propio commit (`chore`).
- Corrige commits locales con `git commit --fixup <sha>` + `git rebase -i --autosquash`.

## 3. Pull Requests pequeñas

**Este es el punto más importante.** Una PR pequeña se revisa bien; una grande se aprueba a ciegas.

| Tamaño (líneas cambiadas) | Estado |
|---|---|
| ≤ 400 líneas y ≤ 10 archivos | Objetivo |
| 401 – 800 | Justificar en la descripción |
| > 800 | Dividir antes de abrir la PR |

Cómo mantenerlas pequeñas:

- Una PR resuelve **un** problema. Refactors previos van en su propia PR.
- Encadena PRs (stacked PRs) cuando una feature es grande.
- Usa feature flags para integrar trabajo incompleto sin exponerlo.
- Archivos generados o lockfiles no cuentan para el límite, pero menciónalos.

Ramas: `<tipo>/<descripción-corta>`, por ejemplo `feat/chat-badges` o `fix/obs-reconnect`.
El título de la PR sigue Conventional Commits (se usa al hacer squash merge).

## Flujo diario

```sh
git switch main && git pull
git switch -c feat/mi-cambio
# ... cambios pequeños, commits atómicos ...
git push -u origin feat/mi-cambio
gh pr create --fill
```

Si trabajas con submódulos, consulta [docs/submodules.md](docs/submodules.md).
