#!/bin/sh
set -eu

selected=false
dependency=false
previous=
for argument in "$@"; do
    if [ "$argument" = "$OPTIC_SELECTED_TARGET_MARKER" ]; then
        selected=true
    fi
    if [ "$previous" = --crate-name ] && [ "$argument" = fixture_dependency ]; then
        dependency=true
    fi
    previous="$argument"
done

if [ "$selected" = true ]; then
    printf '%s\n' "selected-$OPTIC_COMPILER_MODE" >> "$OPTIC_TEST_EVENTS"
elif [ "$dependency" = true ]; then
    printf '%s\n' dependency >> "$OPTIC_TEST_EVENTS"
fi

exec "$OPTIC_TEST_INNER_WRAPPER" "$@"
