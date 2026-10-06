#!/usr/bin/env python3
"""Enforce the crate dependency rule from ARCH.md section 5.3 (G15).

Reads `cargo metadata` and fails when a workspace crate has a normal or build
dependency that its layer does not allow, when a new workspace crate has no
rule yet, or when an inner crate stops forbidding `unsafe` code.
Dev-dependencies are not checked: tests may use fakes from any layer.

Usage: python scripts/check_layers.py
"""

import json
import pathlib
import subprocess
import sys

INNER = {"dusk-domain", "dusk-app"}
ANY = None  # composition roots and spikes may depend on anything

# Allowed normal/build dependencies per crate (workspace and external).
RULES = {
    "dusk-domain": set(),
    "dusk-app": {"dusk-domain", "log"},
    "dusk-mccs": INNER,
    "dusk-ui-model": INNER,
    "dusk-ddc-fake": INNER,
    # Exception (ARCH 5.3): `windows` for the Windows-only named-pipe transport.
    "dusk-ipc": INNER | {"log", "serde", "serde_json", "windows"},
    # The CLI is a client of the daemon, so it reaches it through `ipc`.
    "dusk-cli": INNER | {"dusk-ipc", "serde_json", "clap"},
    "dusk-ddc-windows": INNER | {"dusk-mccs", "log", "windows"},
    "dusk-panel-windows": INNER | {"log", "windows"},
    "dusk-store-file": INNER | {"log", "serde", "toml", "windows"},
    "dusk-ui-win32": INNER | {"dusk-ui-model", "log", "windows"},
    "dusk-bin-cli": ANY,
    "dusk-bin-daemon": ANY,
    "dusk-windows-ddc-spike": ANY,
}

# Crates that must keep `#![forbid(unsafe_code)]` (ARCH section 12).
FORBID_UNSAFE = [
    "dusk-domain",
    "dusk-app",
    "dusk-mccs",
    "dusk-ui-model",
    "dusk-cli",
    "dusk-ddc-fake",
]


def main() -> int:
    metadata = json.loads(
        subprocess.run(
            ["cargo", "metadata", "--format-version", "1", "--no-deps"],
            check=True,
            capture_output=True,
            text=True,
        ).stdout
    )
    errors = []
    packages = {package["name"]: package for package in metadata["packages"]}
    for name, package in sorted(packages.items()):
        if name not in RULES:
            errors.append(f"{name}: no layering rule; add it to scripts/check_layers.py")
            continue
        allowed = RULES[name]
        if allowed is ANY:
            continue
        for dependency in package["dependencies"]:
            if dependency.get("kind") == "dev":
                continue
            if dependency["name"] not in allowed:
                errors.append(f"{name} -> {dependency['name']} is not allowed (ARCH.md 5.3)")

    for name in FORBID_UNSAFE:
        manifest = pathlib.Path(packages[name]["manifest_path"])
        source = (manifest.parent / "src" / "lib.rs").read_text(encoding="utf-8")
        if "#![forbid(unsafe_code)]" not in source:
            errors.append(f"{name}: src/lib.rs must keep #![forbid(unsafe_code)]")

    for error in errors:
        print(f"error: {error}", file=sys.stderr)
    if not errors:
        print(f"layering ok: {len(packages)} workspace crates checked")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
