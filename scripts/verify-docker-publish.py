#!/usr/bin/env python3
"""Fail closed unless a completed CI push on this repository's main is green."""
import json
import os
import re
import sys
from pathlib import Path

REQUIRED_JOBS = {
    "build-guests", "quality", "cuda-quality", "security-audit", "ui", "sdk-wasm-runtime",
    "installer-contract-smoke-test-linux", "installer-contract-smoke-test-windows",
}


def validate(event, pages):
    run = event.get("workflow_run") or {}
    workflow = event.get("workflow") or {}
    repository = event.get("repository") or {}
    head_repo = run.get("head_repository") or {}
    sha = run.get("head_sha", "")
    run_id, run_attempt = run.get("id"), run.get("run_attempt")
    if not (
        workflow.get("name") == "CI"
        and workflow.get("id") is not None
        and workflow.get("id") == run.get("workflow_id")
        and workflow.get("path") == ".github/workflows/ci.yml"
        and run.get("path") == workflow.get("path")
        and run.get("event") == "push"
        and run.get("head_branch") == "main"
        and run.get("status") == "completed"
        and run.get("conclusion") == "success"
        and isinstance(run_id, int) and not isinstance(run_id, bool) and run_id > 0
        and isinstance(run_attempt, int) and not isinstance(run_attempt, bool) and run_attempt > 0
        and repository.get("id") is not None
        and head_repo.get("id") == repository.get("id")
        and re.fullmatch(r"[0-9a-f]{40}", sha)
    ):
        raise ValueError("CI event is not a successful, completed push to this repository's main")
    jobs = [job for page in pages for job in page.get("jobs", [])]
    for name in sorted(REQUIRED_JOBS):
        matches = [j for j in jobs if j.get("name") == name]
        if not matches or any(j.get("run_id") != run_id or
                              not isinstance(j.get("run_attempt"), int) or
                              isinstance(j.get("run_attempt"), bool) or
                              not 1 <= j["run_attempt"] <= run_attempt for j in matches):
            raise ValueError(f"required CI job {name} has missing or invalid run provenance")
        latest_attempt = max(j["run_attempt"] for j in matches)
        latest = [j for j in matches if j["run_attempt"] == latest_attempt]
        if len(latest) != 1 or latest[0].get("status") != "completed" or latest[0].get("conclusion") != "success":
            raise ValueError(f"required CI job {name} is absent, ambiguous or not successful")
    return sha


def main():
    if len(sys.argv) != 2:
        raise ValueError("usage: verify-docker-publish.py <jobs-json>")
    event = json.loads(Path(os.environ["GITHUB_EVENT_PATH"]).read_text())
    pages = json.loads(Path(sys.argv[1]).read_text())
    sha = validate(event, pages)
    with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as output:
        output.write(f"sha={sha}\n")
    print(f"Verified CI for {sha}")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, KeyError, TypeError) as exc:
        print(f"::error::Docker publishing denied: {exc}", file=sys.stderr)
        sys.exit(1)
