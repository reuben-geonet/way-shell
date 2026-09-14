"""Resolve with pinned Nixpkgs, validate every image, then replace the locks."""

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
    args = parser.parse_args()
    root = Path.cwd()
    if not (root / "nix/fedora/config.nix").is_file():
        parser.error("run from the Way-Shell repository root")
    lockdir = root / "nix/fedora/locks"
    lockdir.mkdir(exist_ok=True)
    config = json.loads(args.config.read_text())
    ns = {"repo": "http://linux.duke.edu/metadata/repo"}
    # Keep staging on the same filesystem so os.replace is atomic per file.
    with tempfile.TemporaryDirectory(prefix=".update-", dir=lockdir) as temporary:
        stage = Path(temporary)
        for release, settings in sorted(config["releases"].items()):
            print(f"Resolving Fedora {release}", flush=True)
            packages = sorted(set(config["packages"] + settings.get("extraPackages", [])))
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
            request = stage / f"{release}-request.json"
            write_json(request, {
                "release": release, "packages": packages,
                "archs": config["archs"], "repositories": [repo],
            })
            closure = nix(
                "build", "--file", args.tools / "resolve.nix", "--argstr", "nixpkgs", args.nixpkgs,
                "--argstr", "request", request, "--no-link", "--print-out-paths",
                "--max-jobs", "1", "--cores", "2", "--print-build-logs",
            )
            rpms = json.loads(nix(
                "eval", "--json", "--file", closure,
                "--apply", "f: f { fetchurl = args: args; }",
            ))
            if not rpms or any(set(rpm) != {"url", "sha256"} for rpm in rpms):
                raise ValueError("resolver did not produce SHA-256 RPM fetches")
            manifest = {
                "format": 1, "release": release, "arch": config["arch"],
                "nixpkgsRevision": args.revision, "requestedPackages": packages,
                "repositories": [repo], "rpms": sorted(rpms, key=lambda rpm: rpm["url"]),
            }
            lock = stage / f"{release}.json"
            write_json(lock, manifest)
            print(f"Validating Fedora {release} image ({len(rpms)} RPMs)", flush=True)
            nix(
                "build", "--file", args.tools / "update-image.nix", "--argstr", "nixpkgs", args.nixpkgs,
                "--argstr", "lock", lock, "--no-link", "--max-jobs", "1", "--cores", "2",
                "--print-build-logs",
            )
        # No committed file is replaced until all candidates build successfully.
        for release in sorted(config["releases"]):
            os.replace(stage / f"{release}.json", lockdir / f"{release}.json")
            print(f"Updated nix/fedora/locks/{release}.json", flush=True)


if __name__ == "__main__":
    main()
