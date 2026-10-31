#!/usr/bin/env python3
"""Record non-secret machine, workload and source metadata for one benchmark."""
import json
import os
import platform
import subprocess
from datetime import datetime, timezone
from pathlib import Path


def command(*args):
    try:
        result = subprocess.run(args, capture_output=True, text=True, check=False)
        return result.stdout.strip() if result.returncode == 0 else None
    except OSError:
        return None


def read(path):
    try:
        return Path(path).read_text(encoding="utf-8").strip()
    except OSError:
        return None


def main():
    cpu_info = read("/proc/cpuinfo") or ""
    cpu_model = next((line.split(":", 1)[1].strip() for line in cpu_info.splitlines()
                      if line.startswith("model name") and ":" in line), None)
    data = {
        "captured_at_utc": datetime.now(timezone.utc).isoformat(),
        "commit": command("git", "rev-parse", "HEAD"),
        "github_run_id": os.getenv("GITHUB_RUN_ID"),
        "github_run_attempt": os.getenv("GITHUB_RUN_ATTEMPT"),
        "runner_environment": os.getenv("RUNNER_ENVIRONMENT", "unknown"),
        "platform": {"system": platform.system(), "release": platform.release(),
                     "machine": platform.machine(), "cpu_model": cpu_model,
                     "logical_cpus": os.cpu_count(), "cgroup_cpu_max": read("/sys/fs/cgroup/cpu.max"),
                     "cgroup_memory_max": read("/sys/fs/cgroup/memory.max")},
        "tools": {"docker": command("docker", "--version"), "k3d": command("k3d", "version"),
                  "kubectl": command("kubectl", "version", "--client", "-o", "json"),
                  "fortio": command("fortio", "version")},
        "workload": {"scenario": "faas-chain and repeated cold start; no mesh comparison",
                     "manifest": "benchmark routes re-sealed for this run; not the production manifest",
                     "duration": os.getenv("DURATION", "30s"),
                     "connections": os.getenv("CONNECTIONS", "16"),
                     "target_qps": os.getenv("QPS", "200"),
                     "cold_starts_with_instance_pre": int(os.getenv("COLD_START_SAMPLES", "5")),
                     "cold_starts_without_instance_pre": 5},
        "limitations": ["Runner isolation, hardware class and geographic region are not established; do not treat as the dedicated public comparison profile."],
    }
    output = Path("bench/results/profile.json")
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")
    print(f"Recorded source, runner and workload metadata in {output}")


if __name__ == "__main__":
    main()
