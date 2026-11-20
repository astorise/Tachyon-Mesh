#!/usr/bin/env python3
"""Verify a published release and locally downloaded desktop assets before installation."""
import argparse
import hashlib
import json
import re
from pathlib import Path


def verify(tag, commit, release, pages, manifest_bytes, directory, names):
    if not re.fullmatch(r"v\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?", tag):
        raise ValueError("invalid release tag")
    if not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise ValueError("invalid tag commit")
    if (release.get("tag_name") != tag or release.get("draft") is not False
            or release.get("prerelease") is not ("-" in tag)):
        raise ValueError("release identity, publication state or prerelease flag differs")
    assets = {}
    for page in pages:
        for asset in page:
            name = asset.get("name")
            if not isinstance(name, str) or not name or name in assets:
                raise ValueError("invalid or duplicate release asset")
            assets[name] = asset
    manifest_asset = assets.get("release-manifest.json", {})
    if (manifest_asset.get("state") != "uploaded" or manifest_asset.get("digest") !=
            "sha256:" + hashlib.sha256(manifest_bytes).hexdigest()):
        raise ValueError("release manifest digest mismatch")
    manifest = json.loads(manifest_bytes)
    if (manifest.get("schema_version") != 1 or manifest.get("tag") != tag
            or manifest.get("commit") != commit):
        raise ValueError("release manifest identity mismatch")
    entries = manifest.get("assets")
    if not isinstance(entries, list):
        raise ValueError("invalid release manifest assets")
    recorded = {}
    for entry in entries:
        name = entry.get("name")
        if not isinstance(name, str) or not name or name in recorded:
            raise ValueError("invalid or duplicate manifest asset")
        recorded[name] = entry
    if not names or len(names) != len(set(names)):
        raise ValueError("asset selection must be nonempty and unique")
    for name in names:
        if Path(name).name != name:
            raise ValueError("asset name must not contain a path")
        asset, entry = assets.get(name, {}), recorded.get(name, {})
        digest = entry.get("digest")
        if (not isinstance(digest, str) or not re.fullmatch(r"sha256:[0-9a-f]{64}", digest)
                or asset.get("state") != "uploaded" or asset.get("digest") != digest
                or not isinstance(entry.get("size"), int) or entry["size"] <= 0
                or asset.get("size") != entry["size"]):
            raise ValueError(f"release metadata differs from manifest: {name}")
        path = directory / name
        with path.open("rb") as stream:
            actual = "sha256:" + hashlib.file_digest(stream, "sha256").hexdigest()
        if path.stat().st_size != entry["size"] or actual != digest:
            raise ValueError(f"downloaded asset differs from manifest: {name}")
        print(f"{name}: {digest}")


def main():
    parser = argparse.ArgumentParser()
    for name in ("tag", "commit", "release", "assets", "manifest", "directory"):
        parser.add_argument("--" + name, required=True)
    parser.add_argument("names", nargs="+")
    args = parser.parse_args()
    verify(args.tag, args.commit, json.loads(Path(args.release).read_text(encoding="utf-8")),
           json.loads(Path(args.assets).read_text(encoding="utf-8")),
           Path(args.manifest).read_bytes(), Path(args.directory), args.names)


if __name__ == "__main__":
    main()
