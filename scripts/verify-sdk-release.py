#!/usr/bin/env python3
"""Require a published GitHub release and its matching verified inventory."""
import argparse
import hashlib
import json
import re
from pathlib import Path


def validate(tag, commit, release, pages, manifest_bytes):
    if not re.fullmatch(r"v\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?", tag):
        raise ValueError("invalid release tag")
    if not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise ValueError("invalid commit")
    if release.get("tag_name") != tag or release.get("draft") is not False:
        raise ValueError("the matching GitHub Release must already be published")
    found = [asset for page in pages for asset in page if asset.get("name") == "release-manifest.json"]
    digest = "sha256:" + hashlib.sha256(manifest_bytes).hexdigest()
    if len(found) != 1 or found[0].get("state") != "uploaded" or found[0].get("digest") != digest:
        raise ValueError("published release manifest is missing or its digest differs")
    manifest = json.loads(manifest_bytes)
    if manifest.get("schema_version") != 1 or manifest.get("tag") != tag or manifest.get("commit") != commit:
        raise ValueError("published release manifest does not match the requested tag and commit")


def main():
    parser = argparse.ArgumentParser()
    for name in ("tag", "commit", "release", "assets", "manifest"):
        parser.add_argument("--" + name, required=True)
    args = parser.parse_args()
    validate(args.tag, args.commit, json.loads(Path(args.release).read_text()),
             json.loads(Path(args.assets).read_text()), Path(args.manifest).read_bytes())
    print(f"Published GitHub Release {args.tag} matches commit {args.commit}")


if __name__ == "__main__":
    main()
