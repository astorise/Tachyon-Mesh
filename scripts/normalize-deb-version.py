#!/usr/bin/env python3
"""Give a Tauri-generated Debian package Debian-ordered prerelease metadata."""
import argparse
import os
import re
import subprocess
import tempfile
from pathlib import Path


VERSION = re.compile(r"^(\d+\.\d+\.\d+)(?:-([0-9A-Za-z.-]+))?$")


def package_version(package):
    return subprocess.check_output(
        ["dpkg-deb", "--field", str(package), "Version"], text=True
    ).strip()


def normalize(package, app_version):
    match = VERSION.fullmatch(app_version)
    if not match:
        raise ValueError("invalid application version")
    target = match.group(1) + ("~" + match.group(2) if match.group(2) else "")
    actual = package_version(package)
    if actual == target:
        return target
    if actual != app_version:
        raise ValueError(f"package version {actual} does not match {app_version}")

    # Extracting a package does not run its maintainer scripts. Rebuild the
    # Tauri artifact with root-owned entries, preserving its files and modes.
    with tempfile.TemporaryDirectory(prefix=".tachyon-deb-", dir=package.parent) as tmp:
        directory = Path(tmp) / "package"
        rebuilt = Path(tmp) / "rebuilt.deb"
        subprocess.run(["dpkg-deb", "--raw-extract", str(package), str(directory)], check=True)
        (directory / "DEBIAN").chmod(0o755)
        control = directory / "DEBIAN/control"
        content = control.read_text(encoding="utf-8")
        content, count = re.subn(r"(?m)^Version:[ \t]*[^\r\n]+$", f"Version: {target}", content)
        if count != 1:
            raise ValueError("package control must have exactly one Version field")
        control.write_text(content, encoding="utf-8")
        subprocess.run(["dpkg-deb", "--build", "--root-owner-group", str(directory), str(rebuilt)],
                       check=True)
        if package_version(rebuilt) != target:
            raise ValueError("rebuilt package has the wrong version")
        os.replace(rebuilt, package)
    return target


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--version", required=True)
    parser.add_argument("package", type=Path)
    args = parser.parse_args()
    print(f"Debian package version: {normalize(args.package, args.version)}")


if __name__ == "__main__":
    main()
