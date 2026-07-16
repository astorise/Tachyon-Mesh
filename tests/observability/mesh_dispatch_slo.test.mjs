import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repositoryRoot = resolve(fileURLToPath(new URL("../..", import.meta.url)));
const readRepositoryFile = (path) => readFileSync(resolve(repositoryRoot, path), "utf8");

test("single-node mesh dispatch alert preserves the locality SLO contract", () => {
  const rule = readRepositoryFile("manifests/prometheus-mesh-dispatch-slo.yaml");
  const runbook = readRepositoryFile("docs/mesh-dispatch-locality-slo.md");

  assert.match(rule, /kind: PrometheusRule/);
  assert.match(rule, /alert: MeshDispatchLocalityDegraded/);
  assert.match(rule, /tachyon_mesh_topology="single-node"/);
  assert.match(rule, /reason!="remote"/);
  assert.match(rule, /mode="in_process"/);
  assert.match(rule, /< 0\.95/);
  assert.match(rule, />= 100/);
  assert.match(rule, /for: 10m/);

  assert.match(runbook, /at least 95%/);
  assert.match(runbook, /at least 100 eligible dispatches/);
  assert.match(runbook, /tachyon_mesh_topology: single-node/);
  assert.match(runbook, /reason="remote"/);
});
