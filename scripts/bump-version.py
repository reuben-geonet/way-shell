"""Update the release version in Cargo, RPM and Arch metadata."""

import argparse
from datetime import datetime, timezone
from pathlib import Path
import re


def replace_one(path, pattern, replacement):
    original = path.read_text()
    updated, count = re.subn(pattern, replacement, original, count=1, flags=re.MULTILINE)
    if count != 1:
        raise ValueError(f"expected one version field in {path}")
    path.write_text(updated)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("version", help="new X.Y.Z version")
    args = parser.parse_args()
    version = args.version
    if not re.fullmatch(r"(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)", version):
        parser.error("version must be X.Y.Z")
    root = Path.cwd()
    if not (root / "flake.nix").is_file():
        parser.error("run from the Way-Shell repository root")
    cargo = root / "Cargo.toml"
    replace_one(cargo, r'^(version = ")[^"]+("\n)', rf'\g<1>{version}\g<2>')
    lock = root / "Cargo.lock"
    data = lock.read_text()
    for name in ("way-sh", "way-shell", "way-shell-core", "way-shell-shortcuts"):
        pattern = rf'(\[\[package\]\]\nname = "{re.escape(name)}"\nversion = ")[^"]+("\n)'
        data, count = re.subn(pattern, rf'\g<1>{version}\g<2>', data)
        if count != 1:
            raise ValueError(f"expected one {name} entry in Cargo.lock")
    lock.write_text(data)
    spec = root / "way-shell.spec"
    original = spec.read_text()
    updated = re.sub(r'^Version: .*$', f'Version: {version}', original, count=1, flags=re.MULTILINE)
    updated = re.sub(r'^Release: .*$', 'Release: 1%{?dist}', updated, count=1, flags=re.MULTILINE)
    date = datetime.now(timezone.utc).strftime("%a %b %d %Y")
    updated = updated.replace('%changelog\n', f'%changelog\n* {date} Way Shell contributors - {version}-1\n- Release {version}.\n\n', 1)
    spec.write_text(updated)
    arch = root / "nix/packaging/arch/PKGBUILD.in"
    replace_one(arch, r'^pkgver=.*$', f'pkgver={version}')
    replace_one(arch, r'^pkgrel=.*$', 'pkgrel=1')


if __name__ == "__main__":
    main()
