import copy
import hashlib
import importlib.util
import json
import os
import plistlib
import shutil
import subprocess
import sys
import tempfile
import tomllib
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def load_script(name, filename):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / filename)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


docker_gate = load_script("docker_gate", "verify-docker-publish.py")
release_gate = load_script("release_gate", "verify-release-assets.py")
sdk_gate = load_script("sdk_gate", "verify-sdk-release.py")


class DockerGateTests(unittest.TestCase):
    def setUp(self):
        self.event = {"repository": {"id": 42},
                      "workflow": {"id": 123, "name": "CI", "path": ".github/workflows/ci.yml"},
                      "workflow_run": {
            "name": "", "id": 456, "run_attempt": 1, "workflow_id": 123,
            "path": ".github/workflows/ci.yml", "event": "push",
            "head_branch": "main", "status": "completed", "conclusion": "success",
            "head_repository": {"id": 42}, "head_sha": "a" * 40}}
        self.jobs = [{"jobs": [{"name": name, "run_id": 456, "run_attempt": 1,
                               "status": "completed", "conclusion": "success"}
                              for name in docker_gate.REQUIRED_JOBS]}]

    def test_success_proves_exact_sha(self):
        self.assertEqual(docker_gate.validate(self.event, self.jobs), "a" * 40)

    def test_partial_rerun_uses_latest_attempt_per_job(self):
        self.event["workflow_run"]["run_attempt"] = 2
        security = next(j for j in self.jobs[0]["jobs"] if j["name"] == "security-audit")
        security["conclusion"] = "failure"
        self.jobs[0]["jobs"].append(dict(security, run_attempt=2, conclusion="success"))
        self.assertEqual(docker_gate.validate(self.event, self.jobs), "a" * 40)
        self.jobs[0]["jobs"][-1]["conclusion"] = "failure"
        with self.assertRaisesRegex(ValueError, "security-audit"):
            docker_gate.validate(self.event, self.jobs)

    def test_security_failure_prevents_publication(self):
        self.event["workflow_run"]["conclusion"] = "failure"
        for job in self.jobs[0]["jobs"]:
            if job["name"] == "security-audit":
                job["conclusion"] = "failure"
        with self.assertRaises(ValueError):
            docker_gate.validate(self.event, self.jobs)
        # Defense in depth if the upstream event and job list disagree.
        self.event["workflow_run"]["conclusion"] = "success"
        with self.assertRaisesRegex(ValueError, "security-audit"):
            docker_gate.validate(self.event, self.jobs)

    def test_failed_run_does_not_emit_a_sha_for_the_build_job(self):
        self.event["workflow_run"]["conclusion"] = "failure"
        for job in self.jobs[0]["jobs"]:
            if job["name"] == "security-audit":
                job["conclusion"] = "failure"
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            (directory / "event.json").write_text(json.dumps(self.event))
            (directory / "jobs.json").write_text(json.dumps(self.jobs))
            output = directory / "output"
            env = dict(os.environ, GITHUB_EVENT_PATH=str(directory / "event.json"),
                       GITHUB_OUTPUT=str(output))
            result = subprocess.run([sys.executable, str(ROOT / "scripts/verify-docker-publish.py"),
                                     str(directory / "jobs.json")], env=env, capture_output=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(output.exists())

    def test_fork_pr_missing_or_skipped_checks_are_rejected(self):
        for key, value in [("event", "pull_request"), ("head_branch", "feature"),
                           ("status", "in_progress"), ("path", ".github/workflows/other.yml"),
                           ("head_sha", "a" * 7), ("head_repository", {"id": 43}),
                           ("workflow_id", 124)]:
            event = copy.deepcopy(self.event)
            event["workflow_run"][key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                docker_gate.validate(event, self.jobs)
        jobs = copy.deepcopy(self.jobs)
        jobs[0]["jobs"] = [job for job in jobs[0]["jobs"] if job["name"] != "ui"]
        with self.assertRaisesRegex(ValueError, "ui"):
            docker_gate.validate(self.event, jobs)
        jobs = copy.deepcopy(self.jobs)
        jobs[0]["jobs"][0]["conclusion"] = "skipped"
        with self.assertRaises(ValueError):
            docker_gate.validate(self.event, jobs)

    def test_workflow_connects_gate_to_build_and_promotion(self):
        workflow = (ROOT / ".github/workflows/publish-docker-images.yml").read_text()
        self.assertIn("needs: verify-ci", workflow)
        self.assertIn("needs: [verify-ci, build-images]", workflow)
        self.assertIn("verify-docker-publish.py", workflow)
        self.assertIn('"${image}@${digest}"', workflow)
        self.assertNotIn("publish-docker-images:\n", (ROOT / ".github/workflows/ci.yml").read_text())


class ReleaseInventoryTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.directory = Path(self.tmp.name)
        self.tag = "v1.0.0"
        self.release = {"tag_name": self.tag, "draft": True, "prerelease": False}
        self.assets = []
        self.sample = f"tachyon-mesh-{self.tag}-linux-x86_64.tar.gz"
        expected = release_gate.expected_assets(self.tag, "1.0.0")
        self.assertEqual(len(expected), 107)
        for name in sorted(expected):
            if name.endswith(".sha256"):
                continue
            content = b"valid binary"
            if name.endswith(".json") or name.endswith(".bundle"):
                content = b'{}'
            self.add_asset(name, content)
            if name in {self.sample, self.sample + ".bundle", "integrity-config.schema.json", "integrity-lock.schema.json"}:
                (self.directory / name).write_bytes(content)
        by_name = {asset["name"]: asset for asset in self.assets}
        for name in sorted(expected):
            if name.endswith(".sha256"):
                content = (by_name[name[:-7]]["digest"][7:] + "\n").encode()
                self.add_asset(name, content)
                (self.directory / name).write_bytes(content)

    def add_asset(self, name, content):
        self.assets.append({"name": name, "state": "uploaded", "size": len(content),
                            "digest": "sha256:" + hashlib.sha256(content).hexdigest()})

    def verify(self, assets=None):
        assets = self.assets if assets is None else assets
        # Mimic a paginated GitHub response with more than 100 assets.
        return release_gate.inventory(self.tag, "1.0.0", self.release,
                                      [assets[:100], assets[100:]], self.directory, "a" * 40)

    def test_inventory_accepts_complete_release_and_paginates(self):
        manifest = self.verify()
        self.assertEqual(len(manifest["assets"]), 107)
        self.assertEqual(manifest["commit"], "a" * 40)

    def test_missing_or_extra_asset_blocks_release(self):
        for assets in (self.assets[:-1], self.assets + [{"name": "old-file"}]):
            with self.subTest(count=len(assets)), self.assertRaisesRegex(ValueError, "inventory mismatch"):
                self.verify(assets)

    def test_checksum_and_digest_mismatch_block_release(self):
        (self.directory / (self.sample + ".sha256")).write_text("f" * 64)
        with self.assertRaisesRegex(ValueError, "checksum asset digest mismatch"):
            self.verify()

    def test_archive_digest_must_match_its_checksum(self):
        assets = copy.deepcopy(self.assets)
        archive = next(a for a in assets if a["name"].endswith("-fips.tar.gz"))
        archive["digest"] = "sha256:" + "f" * 64
        with self.assertRaisesRegex(ValueError, "archive checksum mismatch"):
            self.verify(assets)

    def test_sample_signature_and_version_are_required(self):
        (self.directory / (self.sample + ".bundle")).write_bytes(b'corrupt')
        with self.assertRaisesRegex(ValueError, "downloaded asset digest mismatch"):
            self.verify()
        with self.assertRaisesRegex(ValueError, "desktop version"):
            release_gate.expected_assets("v1.0.1", "1.0.0")

    def test_release_candidate_is_marked_as_prerelease_and_version_matches(self):
        self.release["prerelease"] = True
        with self.assertRaisesRegex(ValueError, "prerelease flag"):
            self.verify()
        self.release.update(tag_name="v1.0.0-rc0", prerelease=False)
        with self.assertRaisesRegex(ValueError, "prerelease flag"):
            release_gate.inventory("v1.0.0-rc0", "1.0.0-rc0", self.release,
                                   [], self.directory, "a" * 40)
        self.assertTrue(release_gate.expected_assets("v1.0.0-rc0", "1.0.0-rc0"))
        with self.assertRaisesRegex(ValueError, "desktop version"):
            release_gate.expected_assets("v1.0.0-rc0", "1.0.0")

    def test_macos_build_retains_the_app_archive_required_by_inventory(self):
        for workflow in ("release.yml", "integration.yml"):
            source = (ROOT / ".github/workflows" / workflow).read_text()
            self.assertIn("--target aarch64-apple-darwin --bundles app,dmg", source)
            self.assertIn("target/aarch64-apple-darwin/release/bundle/macos/*.app.tar.gz", source)

    def test_desktop_metadata_versions_match(self):
        config = json.loads((ROOT / "tachyon-ui/tauri.conf.json").read_text())
        npm = json.loads((ROOT / "tachyon-ui/package.json").read_text())
        npm_lock = json.loads((ROOT / "tachyon-ui/package-lock.json").read_text())
        manifest = tomllib.loads((ROOT / "tachyon-ui/Cargo.toml").read_text())
        cargo_lock = tomllib.loads((ROOT / "Cargo.lock").read_text())
        packages = [p for p in cargo_lock["package"] if p["name"] == "tachyon-ui"]
        self.assertEqual(len(packages), 1)
        version = config["version"]
        for actual in (npm["version"], npm_lock["version"],
                       npm_lock["packages"][""]["version"],
                       manifest["package"]["version"], packages[0]["version"]):
            self.assertEqual(actual, version)
        self.assertTrue(release_gate.expected_assets(f"v{version}", version))
        # WiX needs a numeric ProductVersion for an alphabetic prerelease;
        # an explicit override must track the app's major/minor/patch.
        wix = config.get("bundle", {}).get("windows", {}).get("wix", {}).get("version")
        if "-" in version:
            self.assertIsNotNone(wix)
        if wix is not None:
            self.assertRegex(wix, r"^\d+\.\d+\.\d+(?:\.\d+)?$")
            self.assertEqual(wix.split(".")[:3], version.split("-", 1)[0].split("."))
        plist = plistlib.loads((ROOT / "tachyon-ui/Info.plist").read_bytes())
        numeric_version = version.split("-", 1)[0]
        self.assertEqual(plist["CFBundleShortVersionString"], numeric_version)
        self.assertEqual(plist["CFBundleVersion"], numeric_version)
        self.assertEqual(plist["TachyonReleaseVersion"], version)

    def test_unuploaded_or_digestless_asset_blocks_release(self):
        assets = copy.deepcopy(self.assets)
        assets[0]["digest"] = None
        with self.assertRaisesRegex(ValueError, "SHA-256"):
            self.verify(assets)


@unittest.skipUnless(shutil.which("dpkg-deb") and shutil.which("dpkg") and shutil.which("ar"),
                     "requires dpkg and ar tools")
class DebianPrereleaseTests(unittest.TestCase):
    def test_real_deb_orders_rc_before_ga_and_preserves_contents(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            package = directory / "package"
            (package / "DEBIAN").mkdir(parents=True)
            (package / "DEBIAN").chmod(0o755)
            (package / "DEBIAN/control").write_text(
                "Package: tachyon-test\nVersion: 1.0.0-rc0\nArchitecture: all\n"
                "Maintainer: Tachyon <test@example.invalid>\nDescription: test fixture\n")
            payload = package / "usr/share/tachyon-test"
            payload.mkdir(parents=True)
            (payload / "payload.txt").write_text("fixture\n")
            deb = directory / "tachyon.deb"
            subprocess.run(["dpkg-deb", "--build", "--root-owner-group", str(package), str(deb)],
                           check=True, capture_output=True)
            cmd = [sys.executable, str(ROOT / "scripts/normalize-deb-version.py"),
                   "--version", "1.0.0-rc0", str(deb)]
            subprocess.run(cmd, check=True, capture_output=True)
            self.assertEqual(subprocess.check_output(["dpkg-deb", "-f", str(deb), "Version"],
                                                     text=True).strip(), "1.0.0~rc0")
            members = subprocess.check_output(["ar", "t", str(deb)], text=True).splitlines()
            self.assertIn("control.tar.gz", members)
            self.assertIn("data.tar.gz", members)
            subprocess.run(["dpkg", "--compare-versions", "1.0.0~rc0", "lt", "1.0.0"], check=True)
            output = directory / "extracted"
            subprocess.run(["dpkg-deb", "--extract", str(deb), str(output)], check=True)
            self.assertEqual((output / "usr/share/tachyon-test/payload.txt").read_text(), "fixture\n")
            before = hashlib.sha256(deb.read_bytes()).hexdigest()
            subprocess.run(cmd, check=True, capture_output=True)
            self.assertEqual(hashlib.sha256(deb.read_bytes()).hexdigest(), before)
            mismatch = subprocess.run([sys.executable, str(ROOT / "scripts/normalize-deb-version.py"),
                                       "--version", "1.0.1-rc0", str(deb)], capture_output=True)
            self.assertNotEqual(mismatch.returncode, 0)
            self.assertEqual(hashlib.sha256(deb.read_bytes()).hexdigest(), before)
            image = directory / "tachyon.AppImage"
            image.write_bytes(b"appimage fixture")
            stage = directory / "stage"
            stage_command = ["bash", str(ROOT / "scripts/stage-linux-desktop-release.sh"),
                             "1.0.0-rc0", "v1.0.0-rc0", str(deb), str(image), str(stage)]
            subprocess.run(stage_command, check=True, capture_output=True)
            expected = release_gate.expected_assets("v1.0.0-rc0", "1.0.0-rc0")
            self.assertEqual({p.name for p in stage.iterdir()},
                             {name for name in expected if name.endswith(("_amd64.deb", "_amd64.AppImage"))})
            stage_command[3] = "v1.0.0"
            self.assertNotEqual(subprocess.run(stage_command, capture_output=True).returncode, 0)

    def test_reencodes_an_already_normalized_zstd_package(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            package = directory / "package"
            (package / "DEBIAN").mkdir(parents=True)
            (package / "DEBIAN").chmod(0o755)
            (package / "DEBIAN/control").write_text(
                "Package: tachyon-test\nVersion: 1.0.0~rc0\nArchitecture: all\n"
                "Maintainer: Tachyon <test@example.invalid>\nDescription: test fixture\n")
            deb = directory / "tachyon.deb"
            subprocess.run(["dpkg-deb", "--build", "--root-owner-group", "-Zzstd",
                            str(package), str(deb)], check=True, capture_output=True)
            subprocess.run([sys.executable, str(ROOT / "scripts/normalize-deb-version.py"),
                            "--version", "1.0.0-rc0", str(deb)], check=True, capture_output=True)
            members = subprocess.check_output(["ar", "t", str(deb)], text=True).splitlines()
            self.assertIn("control.tar.gz", members)
            self.assertIn("data.tar.gz", members)


class SdkReleaseGateTests(unittest.TestCase):
    def setUp(self):
        self.tag = "v1.0.0"
        self.commit = "a" * 40
        self.release = {"tag_name": self.tag, "draft": False}
        self.manifest = json.dumps({"schema_version": 1, "tag": self.tag,
                                    "commit": self.commit}).encode()
        self.assets = [[{"name": "release-manifest.json", "state": "uploaded",
                         "digest": "sha256:" + hashlib.sha256(self.manifest).hexdigest()}]]

    def test_matching_published_release_is_accepted(self):
        sdk_gate.validate(self.tag, self.commit, self.release, self.assets, self.manifest)

    def test_draft_or_wrong_commit_never_authorizes_sdk_publication(self):
        self.release["draft"] = True
        with self.assertRaisesRegex(ValueError, "already be published"):
            sdk_gate.validate(self.tag, self.commit, self.release, self.assets, self.manifest)
        self.release["draft"] = False
        with self.assertRaisesRegex(ValueError, "does not match"):
            sdk_gate.validate(self.tag, "b" * 40, self.release, self.assets, self.manifest)

    def test_bad_manifest_digest_or_missing_manifest_is_rejected(self):
        self.assets[0][0]["digest"] = "sha256:" + "f" * 64
        with self.assertRaisesRegex(ValueError, "digest differs"):
            sdk_gate.validate(self.tag, self.commit, self.release, self.assets, self.manifest)


if __name__ == "__main__":
    unittest.main()
