#!/usr/bin/env python3
from __future__ import annotations

import json
from pathlib import Path


RAW_DIR = Path("bench/results/raw")
REPORT = Path("bench/results/report.md")


def percentile(data: dict, key: str) -> float | None:
    duration = data.get("DurationHistogram") or data.get("duration_histogram") or {}
    percentiles = duration.get("Percentiles") or duration.get("percentiles") or []
    for item in percentiles:
        if str(item.get("Percentile") or item.get("percentile")) == key:
            return float(item.get("Value") or item.get("value"))
    return None


def main() -> int:
    files = sorted(RAW_DIR.glob("*.json"))
    REPORT.parent.mkdir(parents=True, exist_ok=True)
    if not files:
        REPORT.write_text(
            "# Benchmark Report\n\n"
            "No raw Fortio JSON files were found in `bench/results/raw/`.\n"
            "Run `bench/run-fortio.sh` before publishing benchmark numbers.\n",
            encoding="utf-8",
        )
        return 0

    rows = []
    for path in files:
        data = json.loads(path.read_text(encoding="utf-8"))
        stem = path.stem
        name, qps = stem.rsplit("-", 1)
        rows.append(
            (
                name,
                qps.replace("qps", ""),
                percentile(data, "50") or percentile(data, "50.0"),
                percentile(data, "90") or percentile(data, "90.0"),
                percentile(data, "99") or percentile(data, "99.0"),
            )
        )

    lines = [
        "# Benchmark Report",
        "",
        "| Mesh | Target QPS | p50 latency (s) | p90 latency (s) | p99 latency (s) |",
        "| --- | ---: | ---: | ---: | ---: |",
    ]
    for mesh, qps, p50, p90, p99 in rows:
        lines.append(
            f"| {mesh} | {qps} | {p50 if p50 is not None else 'n/a'} | "
            f"{p90 if p90 is not None else 'n/a'} | {p99 if p99 is not None else 'n/a'} |"
        )

    resources = RAW_DIR / "pod-resources.txt"
    if resources.exists():
        lines.extend(["", "## Kubernetes Resource Snapshot", "", "```text"])
        lines.extend(resources.read_text(encoding="utf-8").splitlines())
        lines.append("```")

    REPORT.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
