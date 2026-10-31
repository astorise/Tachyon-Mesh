import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
REPORT = ROOT / "bench/generate-report.py"
COLD_START = ROOT / "bench/run-cold-start.sh"


class BenchmarkReportingTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.results = self.root / "bench/results"
        self.raw = self.results / "raw"
        self.raw.mkdir(parents=True)
        (self.results / "thresholds.json").write_text('{}')

    def run_report(self, *args):
        return subprocess.run([sys.executable, str(REPORT), *args], cwd=self.root,
                              text=True, capture_output=True)

    def create_regression_data(self, count=5, errors=0):
        fortio = {"DurationHistogram": {"Count": 1000, "Percentiles": [
            {"Percentile": 50, "Value": 0.01}, {"Percentile": 99, "Value": 0.02}]},
            "RetCodes": {"200": 1000 - errors, "500": errors}}
        for label in ("in-process", "forced-transport"):
            (self.raw / f"faas-chain-{label}.json").write_text(json.dumps(fortio))
        for label, size in (("with-instance-pre", count), ("without-instance-pre", 5)):
            (self.raw / f"cold-start-{label}.jsonl").write_text(
                ''.join(json.dumps({"latency_s": i / 1000}) + '\n' for i in range(1, size + 1)))
        (self.raw / "dispatch-log-in-process.txt").write_text('mesh dispatch decision mode="in_process"\n')

    def test_required_run_rejects_missing_data(self):
        result = self.run_report('--require-regression-data')
        self.assertNotEqual(result.returncode, 0)

    def test_five_samples_are_not_presented_as_p99(self):
        self.create_regression_data()
        result = self.run_report('--require-regression-data')
        self.assertEqual(result.returncode, 0, result.stderr)
        report = (self.results / 'report.md').read_text()
        self.assertIn('| with-instance-pre | 5 | 0.003 | n/a |', report)
        self.assertIn('p99 requires at least 100', report)

    def test_a_hundred_samples_produce_a_p99_and_enforce_threshold(self):
        self.create_regression_data(100)
        (self.results / 'thresholds.json').write_text('{"cold_start_with_instance_pre_p99_ms": 50}')
        result = self.run_report('--require-regression-data', '--min-cold-start-samples', '100')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('| with-instance-pre | 100 | 0.05 | 0.099 |',
                      (self.results / 'report.md').read_text())

    def test_failed_http_responses_and_missing_dispatch_do_not_pass(self):
        self.create_regression_data(errors=1)
        (self.raw / 'dispatch-log-in-process.txt').unlink()
        result = self.run_report('--require-regression-data')
        self.assertNotEqual(result.returncode, 0)
        report = (self.results / 'report.md').read_text()
        self.assertIn('non-200 responses', report)
        self.assertIn('missing in-process dispatch log', report)

    def test_enforced_p99_needs_enough_samples(self):
        self.create_regression_data(5)
        (self.results / 'thresholds.json').write_text('{"cold_start_with_instance_pre_p99_ms": 100}')
        self.assertNotEqual(self.run_report('--require-regression-data').returncode, 0)

    def test_failed_cold_start_never_creates_a_sample(self):
        bindir = self.root / 'bin'
        bindir.mkdir()
        curl = bindir / 'curl'
        curl.write_text('#!/bin/sh\nprintf "0.123 500"\nexit 22\n')
        curl.chmod(0o755)
        env = dict(os.environ, PATH=str(bindir) + os.pathsep + os.environ['PATH'],
                   RESULT_DIR=str(self.raw))
        result = subprocess.run(['bash', str(COLD_START)], cwd=ROOT, env=env, capture_output=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse((self.raw / 'cold-start-with-instance-pre.jsonl').exists())

    def test_successful_cold_starts_append_parseable_samples(self):
        bindir = self.root / 'bin'
        bindir.mkdir()
        curl = bindir / 'curl'
        curl.write_text('#!/bin/sh\nprintf "0.123 200"\n')
        curl.chmod(0o755)
        env = dict(os.environ, PATH=str(bindir) + os.pathsep + os.environ['PATH'],
                   RESULT_DIR=str(self.raw))
        for _ in range(2):
            result = subprocess.run(['bash', str(COLD_START)], cwd=ROOT, env=env, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr)
        samples = (self.raw / 'cold-start-with-instance-pre.jsonl').read_text().splitlines()
        self.assertEqual([json.loads(line)['latency_s'] for line in samples], [0.123, 0.123])


if __name__ == '__main__':
    unittest.main()
