#!/usr/bin/env bash

set -euo pipefail

printf '%s\t%s\n' "$1" "$*" >> "${OPTIC_DOCTEST_RUN_LOG:?}"
