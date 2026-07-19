## 1. Alert policy

- [x] 1.1 Add the optional single-node PrometheusRule with the 95% locality objective, 15-minute window, 100-request floor, and 10-minute hold.
- [x] 1.2 Add a runbook documenting the scrape label, PromQL, exclusions, and operator response.

## 2. Product documentation

- [x] 2.1 Add the locality SLO and alert semantics to the canonical compute-observability OpenSpec.

## 3. Verification

- [x] 3.1 Add a focused test that guards the shipped alert expression and runbook contract.
- [x] 3.2 Validate the OpenSpec change and the affected test suite.
