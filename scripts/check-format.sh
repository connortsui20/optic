#!/usr/bin/env bash
# Check both Cargo modules and the runtime-compiled standalone driver.
set -euo pipefail

cd "$(dirname "$0")/.."

cargo fmt --all -- --check

# Include fixture sources that Cargo does not discover. Rustfmt leaves some lines unwrapped.
python3 - <<'PY'
import os
import pathlib
import re
import subprocess
import sys

names = subprocess.check_output([
    "git", "-c", "core.fsmonitor=false", "ls-files", "-z",
    "--cached", "--others", "--exclude-standard", "--", "*.rs",
]).split(b"\0")
paths = [pathlib.Path(os.fsdecode(name)) for name in names if name]
paths = [path for path in paths if path.is_file()]
subprocess.run(
    ["rustfmt", "--edition", "2024", "--check", *map(str, paths)],
    check=True,
)

# Keep exact compiler-source permalinks intact so reviewers can inspect the pinned implementation.
permalink = re.compile(
    r"\s*//[/!]\s+\[[^]]+\]: "
    r"https://github\.com/rust-lang/rust/blob/[0-9a-f]{40}/\S+"
)
failed = False
for path in paths:
    for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        width = len(line.expandtabs(4))
        if width > 100 and not permalink.fullmatch(line):
            print(f"{path}:{number}: Rust lines must fit in 100 columns, got {width}.")
            failed = True
sys.exit(1 if failed else 0)
PY
