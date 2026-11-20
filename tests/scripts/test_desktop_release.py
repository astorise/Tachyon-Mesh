import hashlib
import importlib.util
import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location("desktop_release", Path(__file__).resolve().parents[2] / "scripts" / "verify-desktop-release.py")
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)


class PublishedDesktopReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.directory = Path(self.temp.name)
        self.tag = "v1.0.0-rc1"
        self.sha = "a" * 40
        self.name = "tachyon-studio_1.0.0-rc1_x64.msi"
        (self.directory / self.name).write_bytes(b"installer")
        digest = "sha256:" + hashlib.sha256(b"installer").hexdigest()
        self.entry = {"name": self.name, "size": 9, "digest": digest}
        self.manifest = {"schema_version": 1, "tag": self.tag, "commit": self.sha, "assets": [self.entry]}
        self.release = {"tag_name": self.tag, "draft": False, "prerelease": True}
        self.asset = dict(self.entry, state="uploaded")

    def check(self):
        data = json.dumps(self.manifest).encode()
        manifest_asset = {"name": "release-manifest.json", "state": "uploaded", "digest": "sha256:" + hashlib.sha256(data).hexdigest()}
        checker.verify(self.tag, self.sha, self.release, [[self.asset, manifest_asset]], data, self.directory, [self.name])

    def test_matching_release_and_file(self):
        self.check()

    def test_tampered_download_is_rejected(self):
        (self.directory / self.name).write_bytes(b"tampered!")
        with self.assertRaisesRegex(ValueError, "downloaded asset"):
            self.check()

    def test_mismatched_release_metadata_is_rejected(self):
        self.asset["digest"] = "sha256:" + "0" * 64
        with self.assertRaisesRegex(ValueError, "metadata"):
            self.check()

    def test_wrong_commit_or_draft_is_rejected(self):
        self.manifest["commit"] = "b" * 40
        with self.assertRaisesRegex(ValueError, "identity"):
            self.check()
        self.manifest["commit"] = self.sha
        self.release["draft"] = True
        with self.assertRaisesRegex(ValueError, "publication state"):
            self.check()

    def test_manifest_digest_is_required(self):
        data = json.dumps(self.manifest).encode()
        with self.assertRaisesRegex(ValueError, "manifest digest"):
            checker.verify(self.tag, self.sha, self.release, [[self.asset]], data, self.directory, [self.name])


class DebianVersionTests(unittest.TestCase):
    def test_workflow_version_conversion_preserves_literal_tilde(self):
        workflow = (Path(__file__).resolve().parents[2] / ".github/workflows/qualify-published-desktop.yml").read_text(encoding="utf-8")
        lines = [line.strip() for line in workflow.splitlines() if line.strip().startswith("expected=")]
        self.assertEqual(len(lines), 1)
        for version, expected in (("1.0.0-rc1", "1.0.0~rc1"), ("1.0.0", "1.0.0")):
            with self.subTest(version=version):
                result = subprocess.run(["bash", "-c", lines[0] + '\nprintf "%s" "$expected"'],
                                        env={**os.environ, "version": version, "HOME": "/test-home"},
                                        capture_output=True, text=True, check=True)
                self.assertEqual(result.stdout, expected)


if __name__ == "__main__":
    unittest.main()
