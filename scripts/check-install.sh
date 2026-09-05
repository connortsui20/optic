#!/usr/bin/env bash
# Verify the seven release archives and installed journeys on Linux or macOS.
# Requires Python 3.11+, Git, and the installed toolchain from rust-toolchain.toml.
# Linux also runs the public API consumer and explicitly captures this checkout.
# The temporary root remains available after success or failure for diagnostics.
set -euo pipefail

cd "$(dirname "$0")/.."
checkout=$(pwd -P)
if [[ -n $(git -c core.fsmonitor=false status --porcelain) ]]; then
    echo "Installation verification requires clean committed source." >&2
    exit 1
fi
case $(uname -s) in
    Linux|Darwin) ;;
    *) echo "Installation verification supports Linux and macOS." >&2; exit 1 ;;
esac

cargo_bin=$(rustup which cargo)
toolchain=$(cd "$(dirname "$cargo_bin")/.." && pwd -P)
rustup_root=$(cd "$toolchain/../.." && pwd -P)
root=$(mktemp -d /tmp/optic-install.XXXXXXXX)
root=$(cd "$root" && pwd -P)
echo "Installation evidence: $root"
trap 'echo "Installation verification failed. Evidence: $root" >&2' ERR
mkdir -p "$root/setup-home" "$root/temp" "$root/archives" "$root/prefix"

# Only the installed compiler and ordinary host tools enter the child environment.
child_env=(env -i "PATH=$toolchain/bin:/usr/bin:/bin:/usr/sbin:/sbin"
    "HOME=$root" "CARGO=$cargo_bin" "CARGO_TERM_COLOR=never"
    "RUSTUP_HOME=$rustup_root" "RUSTUP_TOOLCHAIN=$toolchain" RUSTUP_AUTO_INSTALL=0
    "TMPDIR=$root/temp" "TMP=$root/temp" "TEMP=$root/temp"
    "LD_LIBRARY_PATH=$toolchain/lib" "DYLD_LIBRARY_PATH=$toolchain/lib")
for name in SDKROOT DEVELOPER_DIR SYSTEMROOT; do
    if [[ -n ${!name:-} ]]; then
        child_env+=("$name=${!name}")
    fi
done
setup() {
    "${child_env[@]}" "CARGO_HOME=$root/setup-home" \
        "CARGO_TARGET_DIR=$root/setup-target" "$@"
}

git rev-parse HEAD | tee "$root/revision.txt"
setup rustc -vV | tee "$root/compiler.txt"
setup cargo package --workspace --exclude cargo-optic-test-support --locked \
    2>&1 | tee "$root/package.log"

packages=(cargo-optic cargo-optic-api cargo-optic-capture cargo-optic-compiler
    cargo-optic-evidence cargo-optic-records cargo-optic-store)
patches=()
for package in "${packages[@]}"; do
    archive="$root/setup-target/package/$package-0.1.0.crate"
    tar -xzf "$archive" -C "$root/archives"
    patches+=(--config "patch.crates-io.$package.path=\"$root/archives/$package-0.1.0\"")
done

python3 - "$root/archives" <<'PY'
import pathlib
import sys
import tomllib

archives = pathlib.Path(sys.argv[1])
assert len(list(archives.iterdir())) == 7
for package in archives.iterdir():
    manifest = tomllib.loads((package / "Cargo.toml").read_text())
    assert manifest["package"]["version"] == "0.1.0", package

    def dependencies(table):
        for key, value in table.items():
            if key in ("dependencies", "dev-dependencies", "build-dependencies"):
                for name, dependency in value.items():
                    assert name != "cargo-optic-test-support", (package, name)
                    assert "path" not in dependency, (package, name, dependency)
                    assert dependency.get("package") != "cargo-optic-test-support"
            elif isinstance(value, dict):
                dependencies(value)

    dependencies(manifest)
    for source in package.rglob("*.rs"):
        assert "cargo_optic_test_support" not in source.read_text(), source
    print("Normalized archive:", package.name)
PY

# These copies are complete before the runtime journey leaves the checkout.
cp -R scripts/install-fixtures/consumer "$root/consumer"
for journey in cli api; do
    cp -R crates/cargo-optic-test-support/fixtures/default-tracking "$root/$journey-workspace"
    setup git -C "$root/$journey-workspace" init -q
    setup git -C "$root/$journey-workspace" add .
    setup git -C "$root/$journey-workspace" -c user.name=Optic \
        -c user.email=optic@example.invalid commit -qm "Initialize installation fixture"
done
cd "$root"

setup cargo metadata --manifest-path "$root/archives/cargo-optic-0.1.0/Cargo.toml" \
    --format-version 1 "${patches[@]}" > "$root/cli-metadata.json"
if [[ $(uname -s) == Linux ]]; then
    setup cargo metadata --manifest-path "$root/consumer/Cargo.toml" \
        --format-version 1 "${patches[@]}" > "$root/api-metadata.json"
fi
python3 - "$root" <<'PY'
import json
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
expected = {path.name.removesuffix("-0.1.0") for path in (root / "archives").iterdir()}
for metadata in root.glob("*-metadata.json"):
    resolved = json.loads(metadata.read_text())
    seen = set()
    for package in resolved["packages"]:
        name = package["name"]
        assert name != "cargo-optic-test-support", package
        path = pathlib.Path(package["manifest_path"]).resolve()
        if name in expected:
            assert package["version"] == "0.1.0", package
            assert package["source"] is None, package
            assert path == root / "archives" / f"{name}-0.1.0" / "Cargo.toml", package
            seen.add(name)
        elif name == "optic-install-consumer":
            assert path == root / "consumer/Cargo.toml", package
        else:
            assert package["source"].startswith("registry+"), package
    required = expected if metadata.name == "cli-metadata.json" else expected - {"cargo-optic"}
    assert seen == required, (metadata, seen)
    print("Archive-only Optic resolution:", metadata.name)
PY

setup cargo install --path "$root/archives/cargo-optic-0.1.0" --root "$root/prefix" \
    --locked "${patches[@]}" 2>&1 | tee "$root/install.log"
if [[ $(uname -s) == Linux ]]; then
    setup cargo build --manifest-path "$root/consumer/Cargo.toml" --locked "${patches[@]}"
    setup cargo clippy --manifest-path "$root/consumer/Cargo.toml" --locked "${patches[@]}" -- -D warnings
    setup env RUSTDOCFLAGS=-Dwarnings cargo doc --manifest-path "$root/consumer/Cargo.toml" \
        --no-deps --locked "${patches[@]}"
    # Third-party dependencies enter self-hosting through a separate fetch step.
    setup cargo fetch --manifest-path "$checkout/Cargo.toml" --locked
fi

python3 - "$root" "$toolchain" "$rustup_root" "$checkout" <<'PY'
import os
import pathlib
import re
import shutil
import subprocess
import sys

root, toolchain, rustup_root, checkout = map(pathlib.Path, sys.argv[1:])

def environment(journey):
    home = root / f"{journey}-home"
    home.mkdir()
    assert not (home / "optic").exists()
    env = {
        "PATH": f"{root}/prefix/bin:{toolchain}/bin:/usr/bin:/bin:/usr/sbin:/sbin",
        "HOME": str(root), "CARGO": str(toolchain / "bin/cargo"),
        "CARGO_HOME": str(home), "CARGO_TARGET_DIR": str(root / f"{journey}-target"),
        "CARGO_BUILD_BUILD_DIR": str(root / f"{journey}-build"),
        "CARGO_NET_OFFLINE": "true", "CARGO_TERM_COLOR": "never",
        "RUSTUP_HOME": str(rustup_root), "RUSTUP_TOOLCHAIN": str(toolchain),
        "RUSTUP_AUTO_INSTALL": "0", "TMPDIR": str(root / "temp"),
        "TMP": str(root / "temp"), "TEMP": str(root / "temp"),
        "LD_LIBRARY_PATH": str(toolchain / "lib"),
        "DYLD_LIBRARY_PATH": str(toolchain / "lib"),
    }
    for name in ("SDKROOT", "DEVELOPER_DIR", "SYSTEMROOT"):
        if name in os.environ:
            env[name] = os.environ[name]
    return env

def run(arguments, workspace, env):
    result = subprocess.run(arguments, cwd=workspace, env=env, capture_output=True)
    with (root / "journey.log").open("ab") as log:
        log.write(f"cwd: {workspace}\ncommand: {arguments!r}\nstatus: {result.returncode}\n".encode())
        log.write(result.stdout + result.stderr + b"\n")
    if result.returncode:
        sys.stderr.buffer.write(result.stdout + result.stderr)
        raise RuntimeError(f"command failed in {workspace}: {arguments!r}")
    return result.stdout

def cli_journey(workspace, env, package, query, edit):
    def optic(*arguments):
        return run(["cargo", "optic", *arguments], workspace, env)

    def capture(title, *extra):
        output = optic("capture", "-p", package, "--lib", "--release", *extra).decode()
        match = re.search(rf"^{title} ([k-z]{{32}})\n  Completed  (\S+)", output)
        assert match, output
        print(output, flush=True)
        return match.groups()

    def find(capture_id):
        output = optic("find", "--capture", capture_id, query).decode()
        references = re.findall(r"^  Reference   (\S+)$", output, re.M)
        symbols = re.findall(r"^  Symbol      (\S+)$", output, re.M)
        assert len(references) == len(symbols) == 1, output
        assert references[0].startswith(capture_id + ":"), output
        return references[0], symbols[0]

    def show(reference, output):
        return optic("show", "--instance", reference, "--output", output)

    assert run(["/bin/sh", "-c", "command -v cargo-optic"], workspace, env).strip() == \
        os.fsencode(root / "prefix/bin/cargo-optic")
    assert b"0.1.0" in optic("--version")
    original = capture("Captured")
    history = optic("list-captures")
    assert original[0].encode() in history and original[1].encode() in history
    reference, symbol = find(original[0])
    assert capture("Reused") == original
    assert optic("list-captures") == history
    source = show(reference, "source")
    llvm = show(reference, "llvm")
    assert llvm.startswith(b"define "), llvm
    assert f"@{symbol}(".encode() in llvm, llvm
    if not edit:
        assert b"pub fn generate() -> Self" in source, source
        print("Installed self-hosting journey passed", flush=True)
        return

    assert source == b"pub fn captured_value() -> u64 {\n    42\n}", source
    path = workspace / "src/lib.rs"
    path.write_text(path.read_text().replace("    42", "    12345"))
    assert show(reference, "source") == source
    assert show(reference, "llvm") == llvm
    changed = capture("Captured")
    assert changed[0] != original[0]
    changed_ref, _ = find(changed[0])
    assert show(changed_ref, "source") == b"pub fn captured_value() -> u64 {\n    12345\n}"
    assert show(changed_ref, "llvm") != llvm
    assert show(reference, "source") == source
    assert show(reference, "llvm") == llvm
    fresh = capture("Captured", "--fresh")
    assert fresh[0] not in (original[0], changed[0])
    assert capture("Reused") == fresh
    assert len(re.findall(rb"^Capture ", optic("list-captures"), re.M)) == 3
    print("Installed fixture journey passed", flush=True)

cli_journey(root / "cli-workspace", environment("cli"), "default_tracking_fixture",
            "default_tracking_fixture::captured_value", True)
if sys.platform == "linux":
    result = run([str(root / "setup-target/debug/optic-install-consumer")],
                 root / "api-workspace", environment("api"))
    print(result.decode(), flush=True)
    selfhost = environment("selfhost")
    shutil.copytree(root / "setup-home/registry", pathlib.Path(selfhost["CARGO_HOME"]) / "registry")
    print("Self-hosting explicitly uses the checkout:", checkout, flush=True)
    cli_journey(checkout, selfhost, "cargo-optic-records", "CaptureId::generate", False)
PY

echo "Installation verification passed. Evidence: $root"
