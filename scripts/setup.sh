#!/bin/sh
# Configura el clon local: hooks, plantilla de commit y submódulos.
# Idempotente: puede ejecutarse tantas veces como se quiera.
set -eu

cd "$(git rev-parse --show-toplevel)"

git config core.hooksPath .githooks
git config commit.template .gitmessage

git config submodule.recurse true
git config push.recurseSubmodules check
git config diff.submodule log
git config status.submoduleSummary true
git config fetch.recurseSubmodules on-demand

if [ -f .gitmodules ]; then
  git submodule update --init --recursive
fi

chmod +x .githooks/* 2>/dev/null || true

echo "streams-tools listo: hooks activos y submódulos configurados."
