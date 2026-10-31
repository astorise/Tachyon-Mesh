#!/usr/bin/env python3
"""Validate draft release inventory and build its digest manifest."""
import argparse
import hashlib
import json
import re
from pathlib import Path


def expected_assets(tag, ui_version):
    if not re.fullmatch(r"v\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?", tag):
        raise ValueError("release tag must be a version such as v1.0.0")
    if tag[1:] != ui_version:
        raise ValueError(f"desktop version {ui_version} does not match tag {tag}")
    expected = {"integrity-config.schema.json", "integrity-lock.schema.json"}
    platforms = [("linux", "x86_64"), ("linux", "aarch64"),
                 ("darwin", "x86_64"), ("darwin", "aarch64"), ("windows", "x86_64")]
    variants = ("default", "fips", "http3", "security", "ai", "ai-cuda", "no-default")
    for platform, arch in platforms:
        for variant in variants:
            if variant == "ai" and (platform, arch) in {("linux", "aarch64"), ("windows", "x86_64")}:
                continue
            if variant == "ai-cuda" and (platform, arch) != ("linux", "x86_64"):
                continue
            suffix = "" if variant == "default" else f"-{variant}"
            extension = ".zip" if platform == "windows" else ".tar.gz"
            archive = f"tachyon-mesh-{tag}-{platform}-{arch}{suffix}{extension}"
            expected.update((archive, archive + ".sha256", archive + ".bundle"))
            # release.yml produces SBOMs for Linux matrix rows only.
            if platform == "linux":
                expected.add(archive + "-sbom.spdx.json")
    expected.update({
        f"tachyon-studio_{ui_version}_amd64.AppImage",
        f"tachyon-studio_{ui_version}_amd64.deb",
        f"tachyon-studio_{ui_version}_aarch64.dmg",
        # tauri-action also archives and uploads the .app produced for a DMG.
        f"tachyon-studio_{ui_version}_aarch64.app.tar.gz",
        f"tachyon-studio_{ui_version}_x64.msi",
        f"tachyon-studio_{ui_version}_x64.exe",
    })
    return expected


def digest_file(path):
    with path.open("rb") as stream:
        return "sha256:" + hashlib.file_digest(stream, "sha256").hexdigest()


def inventory(tag, ui_version, release, pages, downloaded, commit):
    if release.get("draft") is not True or release.get("tag_name") != tag:
        raise ValueError("release is not the expected draft")
    if release.get("prerelease") is not ("-" in tag):
        raise ValueError("release prerelease flag does not match the tag")
    if not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise ValueError("invalid release commit SHA")
    assets = {}
    for page in pages:
        for asset in page:
            name = asset.get("name")
            if not isinstance(name, str) or not name or name in assets:
                raise ValueError(f"invalid or duplicate asset name: {name}")
            assets[name] = asset
    # A prior finalize attempt may already have uploaded the generated manifest.
    assets.pop("release-manifest.json", None)
    expected = expected_assets(tag, ui_version)
    missing, unexpected = expected - assets.keys(), assets.keys() - expected
    if missing or unexpected:
        raise ValueError(f"asset inventory mismatch; missing={sorted(missing)}, unexpected={sorted(unexpected)}")
    for name, asset in assets.items():
        digest = asset.get("digest") or ""
        if asset.get("state") != "uploaded" or not isinstance(asset.get("size"), int) or asset["size"] <= 0 or not re.fullmatch(r"sha256:[0-9a-f]{64}", digest):
            raise ValueError(f"asset {name} lacks uploaded state, size or SHA-256 digest")
    for name in sorted(expected):
        if name.endswith(".sha256"):
            path = downloaded / name
            if digest_file(path) != assets[name]["digest"]:
                raise ValueError(f"checksum asset digest mismatch: {name}")
            checksum = path.read_text(encoding="utf-8").strip()
            if not re.fullmatch(r"[0-9a-f]{64}", checksum):
                raise ValueError(f"invalid checksum file: {name}")
            if assets[name[:-7]]["digest"] != "sha256:" + checksum:
                raise ValueError(f"archive checksum mismatch: {name[:-7]}")
        if name in {"integrity-config.schema.json", "integrity-lock.schema.json"}:
            path = downloaded / name
            if digest_file(path) != assets[name]["digest"]:
                raise ValueError(f"schema digest mismatch: {name}")
            json.loads(path.read_text(encoding="utf-8"))
    # The default Linux archive and its signature bundle are fetched as a
    # representative post-assembly check. The workflow also verifies cosign.
    sample = f"tachyon-mesh-{tag}-linux-x86_64.tar.gz"
    for name in (sample, sample + ".bundle"):
        if digest_file(downloaded / name) != assets[name]["digest"]:
            raise ValueError(f"downloaded asset digest mismatch: {name}")
    json.loads((downloaded / (sample + ".bundle")).read_text(encoding="utf-8"))
    return {"schema_version": 1, "tag": tag, "commit": commit,
            "assets": [{"name": name, "size": assets[name]["size"], "digest": assets[name]["digest"]}
                       for name in sorted(assets)]}


def main():
    parser = argparse.ArgumentParser()
    for name in ("tag", "ui-version", "release", "assets", "downloaded", "commit", "output"):
        parser.add_argument("--" + name, required=True)
    args = parser.parse_args()
    result = inventory(args.tag, args.ui_version, json.loads(Path(args.release).read_text()),
                       json.loads(Path(args.assets).read_text()), Path(args.downloaded), args.commit)
    Path(args.output).write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(f"Validated {len(result['assets'])} release assets for {args.tag} ({args.commit})")


if __name__ == "__main__":
    main()
