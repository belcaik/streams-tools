#!/bin/sh
# Configura el clon local: plantilla de commit y submódulos.
# Idempotente: puede ejecutarse tantas veces como se quiera.
set -eu

cd "$(git rev-parse --show-toplevel)"

git config commit.template .gitmessage

git config submodule.recurse true
git config push.recurseSubmodules check
git config diff.submodule log
git config status.submoduleSummary true
git config fetch.recurseSubmodules on-demand

if [ -f .gitmodules ]; then
  git submodule update --init --recursive
fi

echo "streams-tools listo: plantilla de commit y submódulos configurados."
