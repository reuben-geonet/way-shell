"""Resolve one dated Arch snapshot and replace locks after both images validate."""

import argparse
import base64
from datetime import date, timedelta
import json
import os
from pathlib import Path
import subprocess
import tempfile
import tarfile
import urllib.error
import urllib.request


ARCHIVE = "https://archive.archlinux.org"


def nix(*args):
    return subprocess.check_output(
        ["nix", *map(str, args), "--option", "allow-import-from-derivation", "false"],
        text=True,
    ).strip()


def prefetch(url):
    result = json.loads(nix("store", "prefetch-file", "--json", url))
    return {"url": url, "sha256": result["hash"]}


def exists(url):
    try:
        with urllib.request.urlopen(urllib.request.Request(url, method="HEAD"), timeout=20):
            return True
    except urllib.error.HTTPError as error:
        if error.code == 404:
            return False
        raise


def choose_snapshot(explicit):
    if explicit:
        snapshot = date.fromisoformat(explicit.replace("/", "-"))
        candidates = [snapshot]
    else:
        latest = date.today() - timedelta(days=1)
        candidates = [latest - timedelta(days=offset) for offset in range(14)]
    for snapshot in candidates:
        base = f"{ARCHIVE}/repos/{snapshot:%Y/%m/%d}"
        if all(exists(f"{base}/{repo}/os/x86_64/{repo}.db") for repo in ("core", "extra")):
            return snapshot
    raise ValueError("no complete Arch snapshot found; pass --snapshot YYYY-MM-DD")


def package_hashes(databases):
    result = {}
    for repo, path in databases.items():
        with tarfile.open(path) as archive:
            for member in archive:
                if not member.name.endswith("/desc"):
                    continue
                lines = archive.extractfile(member).read().decode().splitlines()
                filename = lines[lines.index("%FILENAME%") + 1]
                digest = lines[lines.index("%SHA256SUM%") + 1]
                result[(repo, filename)] = "sha256-" + base64.b64encode(bytes.fromhex(digest)).decode()
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path, required=True)
    parser.add_argument("--nixpkgs", required=True)
    parser.add_argument("--snapshot", help="YYYY-MM-DD (default: newest complete dated snapshot)")
    args = parser.parse_args()
    root = Path.cwd()
    if not (root / "nix/packaging/arch/config.nix").is_file():
        parser.error("run from the Way-Shell repository root")
    config = json.loads(args.config.read_text())
    snapshot = choose_snapshot(args.snapshot)
    stamp = snapshot.strftime("%Y/%m/%d")
    base = f"{ARCHIVE}/repos/{stamp}"
    month = snapshot.strftime("%Y.%m.01")
    bootstrap = prefetch(f"{ARCHIVE}/iso/{month}/archlinux-bootstrap-{month}-x86_64.tar.zst")
    lockdir = root / "nix/packaging/arch/locks"
    lockdir.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".update-", dir=lockdir) as temporary:
        stage = Path(temporary)
        databases = {}
        database_paths = {}
        for repo in ("core", "extra"):
            url = f"{base}/{repo}/os/{config['arch']}/{repo}.db"
            entry = prefetch(url)
            databases[repo] = entry
            database_paths[repo] = Path(json.loads(nix("store", "prefetch-file", "--json", url))["storePath"])
        hashes = package_hashes(database_paths)
        locks = []
        for kind, roots in (("build", config["buildPackages"]), ("runtime", config["runtimePackages"])):
            workspace = stage / kind
            sync = workspace / "db/sync"
            sync.mkdir(parents=True)
            for repo, entry in databases.items():
                (sync / f"{repo}.db").write_bytes(database_paths[repo].read_bytes())
            pacman_config = workspace / "pacman.conf"
            pacman_config.write_text(
                "[options]\nArchitecture = x86_64\nSigLevel = Never\n" +
                "\n".join(f"[{repo}]\nServer = {base}/{repo}/os/x86_64" for repo in databases) + "\n"
            )
            plan = subprocess.check_output([
                "pacman", "--config", str(pacman_config), "--dbpath", str(workspace / "db"),
                "--print-format", "%r %f", "-Sp", *roots,
            ], text=True)
            packages = []
            for line in sorted(set(plan.splitlines())):
                repo, filename = line.split(" ", 1)
                if repo not in databases or not filename.endswith(".pkg.tar.zst"):
                    raise ValueError(f"unexpected package plan: {line}")
                packages.append({
                    "url": f"{base}/{repo}/os/{config['arch']}/{filename}",
                    "sha256": hashes[(repo, filename)],
                })
            lock = stage / f"{kind}.json"
            lock.write_text(json.dumps({
                "format": 1, "kind": kind, "arch": config["arch"],
                "snapshot": stamp, "requestedPackages": sorted(roots),
                "bootstrap": bootstrap, "databases": databases, "packages": packages,
            }, indent=2, sort_keys=True) + "\n")
            locks.append(lock)
            print(f"Validating Arch {kind} image ({len(packages)} packages)", flush=True)
            nix("build", "--file", root / "nix/packaging/arch/update-image.nix",
                "--argstr", "nixpkgs", args.nixpkgs, "--argstr", "lock", lock,
                "--no-link", "--max-jobs", "1", "--cores", "2", "--print-build-logs")
        for lock in locks:
            destination = lockdir / lock.name
            os.replace(lock, destination)
            print(f"Updated {destination}", flush=True)


if __name__ == "__main__":
    main()
