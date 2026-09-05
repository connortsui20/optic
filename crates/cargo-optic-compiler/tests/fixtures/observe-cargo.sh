#!/bin/sh
set -eu

if [ -n "${RUSTC_WRAPPER:-}" ]; then
    export OPTIC_TEST_INNER_WRAPPER="$RUSTC_WRAPPER"
    export RUSTC_WRAPPER="$OPTIC_TEST_OBSERVER"
fi

exec "$OPTIC_TEST_REAL_CARGO" "$@"
