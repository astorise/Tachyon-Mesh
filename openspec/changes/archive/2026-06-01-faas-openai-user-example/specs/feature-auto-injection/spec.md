## MODIFIED Requirements

### Requirement: Feature routes auto-injected at startup
The system SHALL call `inject_feature_routes` on every `IntegrityConfig` before
passing it to `build_runtime_state`, both at cold start (`serve_host`) and on
hot-reload (`reload_runtime_from_disk`). Routes already present in the config
MUST NOT be duplicated. When built with `--features ai-inference`, the injected
AI bundle SHALL contain only `/system/model-broker`; it SHALL NOT inject
`/system/ai-list-model` or `/system/ai-openai-adapter`, which are no longer
system FaaS — the OpenAI surface and model registry are the `guest-openai` user
FaaS example (see `openai-compatible-faas`).

#### Scenario: ai-inference binary activates the broker route only
- **WHEN** `core-host` is built with `--features ai-inference` and starts with a manifest that does not contain `/system/model-broker`
- **THEN** the runtime contains `/system/model-broker` as a `role=system` route
- **THEN** the runtime does NOT contain `/system/ai-list-model` or `/system/ai-openai-adapter`

#### Scenario: Injection is idempotent
- **WHEN** `inject_feature_routes` is called on a manifest that already contains `/system/model-broker`
- **THEN** no duplicate route is added and the config is unchanged

#### Scenario: s3-persistence routes injected
- **WHEN** `core-host` is built with `--features s3-persistence`
- **THEN** the runtime contains `/system/s3-proxy` and `/system/storage-broker`

#### Scenario: Default build (no extra features) is unaffected
- **WHEN** `core-host` is built without `ai-inference` or `s3-persistence`
- **THEN** no additional routes are injected
