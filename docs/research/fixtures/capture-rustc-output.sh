#!/usr/bin/env bash

set -euo pipefail

compiler=$1
shift

capture_dir=${OPTIC_RUSTC_CAPTURE_DIR:?}

"$compiler" "$@" \
    > >(tee "$capture_dir/$$.stdout") \
    2> >(tee "$capture_dir/$$.stderr" >&2)
