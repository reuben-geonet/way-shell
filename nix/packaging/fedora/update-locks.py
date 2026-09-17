"""Resolve with DNF, validate every image, then replace the pinned RPM locks."""

import argparse
import base64
import json
import os
from pathlib import Path
import subprocess
import tempfile
import xml.etree.ElementTree as ET


def nix(*args):
    return subprocess.check_output(
        ["nix", *map(str, args), "--option", "allow-import-from-derivation", "false"],
        text=True,
    ).strip()


def prefetch(url, sha256=None):
    args = ["store", "prefetch-file", "--json", url]
    if sha256:
        args += ["--expected-hash", "sha256-" + base64.b64encode(bytes.fromhex(sha256)).decode()]
    return json.loads(nix(*args))


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", required=True, type=Path)
    parser.add_argument("--nixpkgs", required=True)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--tools", required=True, type=Path)
    parser.add_argument("--spec", required=True, type=Path)
    args = parser.parse_args()
    root = Path.cwd()
    if not (root / "nix/packaging/fedora/config.nix").is_file():
        parser.error("run from the Way-Shell repository root")
    lockdir = root / "nix/packaging/fedora/locks"
    lockdir.mkdir(exist_ok=True)
    config = json.loads(args.config.read_text())
    ns = {
        "repo": "http://linux.duke.edu/metadata/repo",
        "rpm": "http://linux.duke.edu/metadata/common",
    }
    # Keep staging on the same filesystem so os.replace is atomic per file.
    with tempfile.TemporaryDirectory(prefix=".update-", dir=lockdir) as temporary:
        stage = Path(temporary)
        locks = []
        for release, settings in sorted(config["releases"].items()):
            print(f"Resolving Fedora {release}", flush=True)
            workspace = stage / release
            (workspace / "root").mkdir(parents=True)
            base_url = settings["baseUrl"]
            repomd_url = base_url + "/repodata/repomd.xml"
            repomd = prefetch(repomd_url)
            xml = ET.parse(repomd["storePath"])
            primary = xml.find("repo:data[@type='primary']", ns)
            checksum = primary.find("repo:checksum", ns)
            if checksum.attrib["type"] != "sha256":
                raise ValueError("repository primary metadata must use SHA-256")
            repo = {
                "baseUrl": base_url,
                "repomd": {"url": repomd_url, "hash": repomd["hash"]},
                "primary": {
                    "url": base_url + "/" + primary.find("repo:location", ns).attrib["href"],
                    "sha256": checksum.text,
                    "timestamp": primary.findtext("repo:timestamp", namespaces=ns),
                },
            }
            # Read checksums from Fedora metadata, not from a second package list.
            primary_path = prefetch(repo["primary"]["url"], checksum.text)["storePath"]
            metadata = workspace / "primary.xml"
            subprocess.run(["zstd", "-dq", primary_path, "-o", metadata], check=True)
            hashes = {}
            for _, package in ET.iterparse(metadata, events=("end",)):
                if package.tag == "{" + ns["rpm"] + "}package":
                    digest = package.find("rpm:checksum", ns)
                    if digest.attrib["type"] != "sha256":
                        raise ValueError("RPM metadata must use SHA-256")
                    url = base_url + "/" + package.find("rpm:location", ns).attrib["href"]
                    hashes[Path(url).name] = {"url": url, "sha256": digest.text}
                    package.clear()
            # An empty root and explicit configuration keep the host out of resolution.
            dnf = [
                "dnf5", "--config=/dev/null", f"--installroot={workspace}/root",
                f"--releasever={release}", f"--forcearch={config['arch']}",
                "--setopt=reposdir=", "--setopt=plugins=False",
                f"--setopt=cachedir={workspace}/cache", f"--setopt=logdir={workspace}/logs",
                f"--setopt=persistdir={workspace}/state", f"--repofrompath=fedora,{base_url}",
            ]
            recommendations = subprocess.check_output([
                "rpmspec", "--define", f"fedora {release}", "--define", f"dist .fc{release}",
                "--query", "--recommends", args.spec,
            ], text=True).splitlines()
            for kind, roots in {
                "build": config["packages"],
                "runtime": config["runtimePackages"],
                "services": recommendations,
            }.items():
                packages = sorted(set(roots))
                transaction = workspace / kind
                if packages:
                    # Store a solved transaction without installing anything. A plain
                    # `dnf download --resolve` can include conflicting providers.
                    subprocess.run(dnf + [
                        f"--setopt=install_weak_deps={'True' if kind == 'services' else 'False'}",
                        "--assumeyes", "install", f"--store={transaction}", *packages,
                    ], check=True)
                rpms = [hashes[path.name] for path in sorted((transaction / "packages").glob("*.rpm"))]
                manifest = {
                    "format": 1, "release": release, "arch": config["arch"],
                    "nixpkgsRevision": args.revision, "requestedPackages": packages,
                    "repositories": [repo], "rpms": sorted(rpms, key=lambda rpm: rpm["url"]),
                }
                lock = stage / f"{release}-{kind}.json"
                write_json(lock, manifest)
                locks.append(lock)
                # Service packages are repository candidates, not a preinstalled image.
                # The installation test validates their DNF transaction with Way-Shell.
                if kind != "services":
                    print(f"Validating Fedora {release}-{kind} image ({len(rpms)} RPMs)", flush=True)
                    nix(
                        "build", "--file", args.tools / "update-image.nix", "--argstr", "nixpkgs", args.nixpkgs,
                        "--argstr", "lock", lock, "--arg", "size", "16384" if kind == "build" else "4096",
                        "--no-link", "--max-jobs", "1", "--cores", "2", "--print-build-logs",
                    )
        # No committed file is replaced until every candidate image builds.
        for lock in locks:
            os.replace(lock, lockdir / lock.name)
            print(f"Updated nix/packaging/fedora/locks/{lock.name}", flush=True)


if __name__ == "__main__":
    main()
