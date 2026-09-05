#!/usr/bin/env bash

set -euo pipefail

wrapper_name=${0##*/}

printf '%s\t%s\t%s\n' "$wrapper_name" "$1" "$*" >> "${OPTIC_WRAPPER_LOG:?}"

exec "$@"
