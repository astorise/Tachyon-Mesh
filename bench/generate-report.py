#!/usr/bin/env python3
from __future__ import annotations

import json
import math
from pathlib import Path


RAW_DIR = Path("bench/results/raw")
REPORT = Path("bench/results/report.md")
THRESHOLDS = Path("bench/results/thresholds.json")


def percentile(data: dict, key: str) -> float | None:
    duration = data.get("DurationHistogram") or data.get("duration_histogram") or {}
    percentiles = duration.get("Percentiles") or duration.get("percentiles") or []
    for item in percentiles:
        if str(item.get("Percentile") or item.get("percentile")) == key:
            return float(item.get("Value") or item.get("value"))
    return None


def sample_percentile(samples: list[float], pct: float) -> float | None:
    if not samples:
        return None
    ordered = sorted(samples)
    index = min(len(ordered) - 1, max(0, math.ceil(pct / 100 * len(ordered)) - 1))
    return ordered[index]


def load_cold_start_samples(label: str) -> list[float]:
    path = RAW_DIR / f"cold-start-{label}.jsonl"
    if not path.exists():
        return []
    samples = []
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line:
            continue
        samples.append(float(json.loads(line)["latency_s"]))
    return samples


def dispatch_mode_share(label: str) -> tuple[int, int] | None:
    """Returns (in_process_count, total_count) parsed from the debug-level
    `mesh dispatch decision` log lines captured via `kubectl logs` (see
    mesh_dispatch_metrics.rs), or None if the log file wasn't captured."""
    path = RAW_DIR / f"dispatch-log-{label}.txt"
    if not path.exists():
        return None
    in_process = 0
    total = 0
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        if "mesh dispatch decision" not in line:
            continue
        total += 1
        if 'mode="in_process"' in line:
            in_process += 1
    return (in_process, total)


def load_thresholds() -> dict:
    if not THRESHOLDS.exists():
        return {}
    return json.loads(THRESHOLDS.read_text(encoding="utf-8"))


def check_threshold(
    lines: list[str],
    label: str,
    observed: float | None,
    threshold: float | None,
    unit: str,
    direction: str = "max",
) -> bool:
    """Appends a status line to `lines`; returns True iff this check failed.

    `direction="max"` fails when `observed > threshold` (e.g. a p99 latency
    ceiling); `direction="min"` fails when `observed < threshold` (e.g. a
    minimum required in-process dispatch share)."""
    if observed is None:
        lines.append(f"- {label}: no data collected (n/a)")
        return False
    if threshold is None:
        lines.append(
            f"- {label}: {observed:.2f}{unit} (provisional — not yet enforced, "
            f"set a value in `{THRESHOLDS}` to start gating)"
        )
        return False
    failed = observed > threshold if direction == "max" else observed < threshold
    comparator = ">" if direction == "max" else "<"
    ok_comparator = "<=" if direction == "max" else ">="
    if failed:
        lines.append(
            f"- {label}: {observed:.2f}{unit} {comparator} threshold {threshold}{unit} — **FAIL**"
        )
        return True
    lines.append(f"- {label}: {observed:.2f}{unit} {ok_comparator} threshold {threshold}{unit} (OK)")
    return False


def main() -> int:
    REPORT.parent.mkdir(parents=True, exist_ok=True)
    all_files = sorted(RAW_DIR.glob("*.json")) if RAW_DIR.exists() else []
    # faas-chain-*.json is handled by its own section below, not the generic
    # per-mesh echo comparison table.
    files = [p for p in all_files if not p.stem.startswith("faas-chain-")]

    lines = ["# Benchmark Report", ""]
    any_data = bool(all_files) or (RAW_DIR / "cold-start-with-instance-pre.jsonl").exists()

    if not any_data:
        REPORT.write_text(
            "\n".join(lines)
            + (
                "\nNo raw Fortio JSON files were found in `bench/results/raw/`.\n"
                "Run `bench/run-fortio.sh` before publishing benchmark numbers.\n"
            ),
            encoding="utf-8",
        )
        return 0

    if files:
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

        lines += [
            "| Mesh | Target QPS | p50 latency (s) | p90 latency (s) | p99 latency (s) |",
            "| --- | ---: | ---: | ---: | ---: |",
        ]
        for mesh, qps, p50, p90, p99 in rows:
            lines.append(
                f"| {mesh} | {qps} | {p50 if p50 is not None else 'n/a'} | "
                f"{p90 if p90 is not None else 'n/a'} | {p99 if p99 is not None else 'n/a'} |"
            )

    thresholds = load_thresholds()
    any_failure = False

    # ── FaaS chain scenario (issue #308) ────────────────────────────────────
    chain_variants = {}
    for label in ("in-process", "forced-transport"):
        path = RAW_DIR / f"faas-chain-{label}.json"
        if path.exists():
            data = json.loads(path.read_text(encoding="utf-8"))
            chain_variants[label] = (
                percentile(data, "50") or percentile(data, "50.0"),
                percentile(data, "99") or percentile(data, "99.0"),
            )

    if chain_variants:
        lines += ["", "## FaaS Chain (guest-chain-a -> b -> c)", ""]
        lines += [
            "| Variant | p50 latency (s) | p99 latency (s) |",
            "| --- | ---: | ---: |",
        ]
        for label, (p50, p99) in chain_variants.items():
            lines.append(
                f"| {label} | {p50 if p50 is not None else 'n/a'} | "
                f"{p99 if p99 is not None else 'n/a'} |"
            )

        lines += ["", "### Thresholds", ""]
        in_process_p99 = chain_variants.get("in-process", (None, None))[1]
        observed_ms = in_process_p99 * 1_000 if in_process_p99 is not None else None
        any_failure |= check_threshold(
            lines,
            "faas_chain_in_process_p99_ms",
            observed_ms,
            thresholds.get("faas_chain_in_process_p99_ms"),
            "ms",
        )

    dispatch_counts = dispatch_mode_share("in-process")
    if dispatch_counts is not None:
        in_process, total = dispatch_counts
        share = (100.0 * in_process / total) if total else None
        lines += [
            "",
            f"Mesh dispatch log: {in_process}/{total} decisions were `mode=\"in_process\"` "
            f"during the in-process chain phase.",
        ]
        any_failure |= check_threshold(
            lines,
            "faas_chain_in_process_share_min_pct",
            share,
            thresholds.get("faas_chain_in_process_share_min_pct"),
            "%",
            direction="min",
        )

    # ── Cold start scenario (issue #308) ────────────────────────────────────
    cold_start_variants = {}
    for label in ("with-instance-pre", "without-instance-pre"):
        samples = load_cold_start_samples(label)
        if samples:
            cold_start_variants[label] = (
                sample_percentile(samples, 50),
                sample_percentile(samples, 99),
            )

    if cold_start_variants:
        lines += ["", "## Cold Start (/api/guest-example after a cache-clearing restart)", ""]
        lines += [
            "| Variant | p50 latency (s) | p99 latency (s) |",
            "| --- | ---: | ---: |",
        ]
        for label, (p50, p99) in cold_start_variants.items():
            lines.append(
                f"| {label} | {p50 if p50 is not None else 'n/a'} | "
                f"{p99 if p99 is not None else 'n/a'} |"
            )

        lines += ["", "### Thresholds", ""]
        with_instance_pre_p99 = cold_start_variants.get("with-instance-pre", (None, None))[1]
        observed_ms = (
            with_instance_pre_p99 * 1_000 if with_instance_pre_p99 is not None else None
        )
        any_failure |= check_threshold(
            lines,
            "cold_start_with_instance_pre_p99_ms",
            observed_ms,
            thresholds.get("cold_start_with_instance_pre_p99_ms"),
            "ms",
        )

    resources = RAW_DIR / "pod-resources.txt"
    if resources.exists():
        lines.extend(["", "## Kubernetes Resource Snapshot", "", "```text"])
        lines.extend(resources.read_text(encoding="utf-8").splitlines())
        lines.append("```")

    REPORT.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return 1 if any_failure else 0


if __name__ == "__main__":
    raise SystemExit(main())
