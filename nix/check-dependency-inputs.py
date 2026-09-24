#!/usr/bin/env python3
"""Check cache boundaries; --build / --fedora VERSION also verify warm reuse."""
import argparse
import json
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent.parent


def nix(*args):
    return subprocess.check_output(
        ["nix", "--option", "allow-import-from-derivation", "false", *args], text=True
    )


def paths(root):
    select = """p: builtins.listToAttrs (map
      (name: { inherit name; value = p.${name}.drvPath; })
      (builtins.filter (name:
        builtins.match "(cargo-deps-native|cargo-deps-fedora-[0-9]+|way-shell|rpm-fedora-[0-9]+)" name != null
      ) (builtins.attrNames p)))"""
    return json.loads(nix("eval", "--json", f"path:{root}#packages.x86_64-linux", "--apply", select))


def check_reuse(root, release=None):
    package = f"rpm-fedora-{release}" if release else "way-shell"
    target = f"path:{root}#{package}"
    result = root / f"result-{package}"
    nix("build", target, "--out-link", str(result))
    try:
        log = (result / "logs/build.log").read_text() if release else nix("log", target)
    except subprocess.CalledProcessError:
        # A restored store output need not include its build log. Rebuild the
        # application against its retained dependencies to obtain diagnostics.
        nix("build", "--rebuild", target, "--no-link")
        log = (result / "logs/build.log").read_text() if release else nix("log", target)
    log = re.sub(r"\x1b\[[0-9;]*m", "", log)
    for crate in ("gtk4", "glib", "wireplumber"):
        assert re.search(rf"Fresh {crate} v", log), f"{crate} artifacts were not reused"
    compiled = set(re.findall(r"Compiling ([\w-]+) v", log))
    workspace = {"way-shell", "way-sh", "way-shell-core", "way-shell-shortcuts"}
    assert compiled == workspace, f"expected only real workspace crates to compile: {compiled}"

    assert "crates/shell/build.rs" in log, "real shell build script was not compiled"
    if not release:
        for binary in ("way-shell", "way-sh"):
            help_text = subprocess.check_output([result / "bin" / binary, "--help"], text=True)
            assert "Usage:" in help_text, f"{binary} is not a real application binary"
    print(f"PASS {package} artifact reuse and real workspace compilation", flush=True)


def change_fedora_rpm(contents):
    manifest = json.loads(contents)
    manifest["rpms"][0]["sha256"] = "0" * 64
    return json.dumps(manifest)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--build", action="store_true")
    parser.add_argument("--fedora", action="append", default=[], metavar="VERSION")
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="way-shell-cache-check-") as temporary:
        root = Path(temporary)
        # Copy tracked working-tree contents, including uncommitted edits, but
        # never target/, VM images or local caches.
        files = subprocess.check_output(["git", "ls-files", "-z"], cwd=ROOT).decode().split("\0")
        for name in filter(None, files):
            destination = root / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / name, destination)
        baseline = paths(root)
        all_outputs = set(baseline)
        native = {"cargo-deps-native", "way-shell"}
        fedora = all_outputs - native
        applications = {name for name in all_outputs if name.startswith("rpm-fedora-")} | {"way-shell"}
        if args.build:
            check_reuse(root)
        fedora_dependencies = {
            release: nix("eval", "--raw", f"path:{root}#cargo-deps-fedora-{release}.drvPath")
            for release in args.fedora
        }
        for release in args.fedora:
            check_reuse(root, release)
        cases = [
            ("crates/shell/src/main.rs", lambda s: s + "\n// cache regression probe\n", applications),
            ("crates/shell/build.rs", lambda s: s + "\n// build script probe\n", applications),
            ("data/theme/way-shell-dark.css", lambda s: s + "\n/* resource probe */\n", applications),
            ("nix/install-tests/fedora/install.sh", lambda s: s + "\n# installation test probe\n", set()),
            ("README.md", lambda s: s + "\nDocumentation probe.\n", set()),
            ("Cargo.lock", lambda s: s + "\n# lock probe\n", all_outputs),
            (
                "Cargo.toml",
                lambda s: s.replace('features = ["v1_4"]', 'features = ["v1_5"]'),
                all_outputs,
            ),
            (
                "Cargo.toml",
                lambda s: s.replace("[profile.release]\ndebug = 1", "[profile.release]\ndebug = 2"),
                all_outputs,
            ),
            ("way-shell.spec", lambda s: s + "\n# spec probe\n", fedora),
            ("nix/cargo.nix", lambda s: s.replace("        dconf", "        sqlite"), native),
            (
                "nix/cargo.nix",
                lambda s: s.replace(
                    "inputs.crane.mkLib pkgs;",
                    '(inputs.crane.mkLib pkgs).overrideScope (_: _: { '
                    'rustc = pkgs.rustc.overrideAttrs (_: { pname = "rustc-probe"; }); });',
                ),
                native,
            ),
            (
                "nix/cargo.nix",
                lambda s: s.replace('CARGO_INCREMENTAL = "0"', 'CARGO_INCREMENTAL = "1"'),
                native,
            ),
        ]
        for package in sorted(applications - {"way-shell"}):
            release = package.removeprefix("rpm-fedora-")
            cases.append((
                f"nix/packaging/fedora/locks/{release}-build.json",
                change_fedora_rpm,
                {name for name in fedora if name.endswith(f"fedora-{release}")},
            ))
        for filename, transform, expected in cases:
            file = root / filename
            original = file.read_text()
            changed = transform(original)
            assert changed != original, f"probe no longer matches {filename}"
            try:
                file.write_text(changed)
                actual = paths(root)
                invalidated = {name for name in baseline if actual[name] != baseline[name]}
                assert invalidated == expected, (filename, invalidated, expected)
                if filename == "crates/shell/src/main.rs":
                    if args.build:
                        check_reuse(root)
                    for release in args.fedora:
                        dependency = nix("eval", "--raw", f"path:{root}#cargo-deps-fedora-{release}.drvPath")
                        assert dependency == fedora_dependencies[release], "application edit invalidated Fedora dependencies"
                        check_reuse(root, release)
            finally:
                file.write_text(original)
            print(f"PASS {filename}", flush=True)


if __name__ == "__main__":
    main()
