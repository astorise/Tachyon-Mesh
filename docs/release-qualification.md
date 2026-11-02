# Release qualification

A green pull request or a successful build is not, by itself, approval to
announce a release. Qualify the **same final commit** for every item below;
record the run URLs, artifact names and digests with the release evidence.
Do not treat skipped checks as passed or treat a release draft as published.

1. Require the `push/main` CI run and k3d integration for the exact final SHA
   to succeed. Check QUIC and gRPC interoperability separately if those
   transports are in the release scope. The tag gate also checks the named
   quality, CUDA, security, guest, SDK-runtime and installer jobs.
2. The `Publish Docker images` workflow runs only after that CI run succeeds.
   It validates the run and critical job conclusions, checks out the tested
   SHA, and builds each variant under a full-SHA tag. Only after all variants
   succeed does it promote their recorded digests to `latest*`, `main*` and
   short-SHA aliases while that SHA is still the main tip. Registry tag updates
   are not atomic across variants; review the publishing run before announcing
   or consuming a newly promoted set. If `main` advances during a promotion,
   a newer successful run must promote its own set.
3. After authorization to create a release tag, run the tag pipeline. Before
   publication it requires a draft containing the server matrix, checksums,
   cosign bundles, Linux server SBOMs, integrity schemas and Linux, macOS and
   Windows desktop installers. It checks all archive checksum files against
   GitHub's recorded digests, downloads and checks the default Linux archive,
   its bundle and the schemas, verifies that archive's cosign identity, and
   attaches `release-manifest.json`. This is not a substitute for testing all
   platforms or every signature. Desktop bundles are not signed by this cosign
   mechanism. The installer verifies SHA-256 and verifies cosign only when it
   is installed.
4. Test the published server installers on clean Linux and Windows machines,
   inspect the manifest and provenance, start the server and make a functional
   request. Resolve and verify the matching WIT artifact from GHCR. A successful
   dry run or an attached CI artifact does not establish these results.
5. The `sdk-wasm-runtime` CI job runs Python and Go guests using their real
   `kv-partition` SDKs against Tachyon. Check that this job succeeded on the
   final SHA, then install and exercise all four published SDKs from clean
   consumer projects. Go consumers need the matching `sdk/go/vX.Y.Z` module
   tag. Until the registry installations succeed, do not claim that the full
   multi-language GA has been qualified.
6. Publish the SDKs using the manually dispatched `Publish SDKs` workflow for
   the same release tag. It requires the published GitHub Release and a
   digest-verified manifest for the resolved commit. Record each package version and registry digest, then
   run the clean consumer checks. The GitHub Release, GHCR and language
   registries are separate operations: partial success is possible, published
   versions may be immutable and there is no automatic rollback. Retry only the
   failed publish after checking the relevant registry's state.
7. Performance claims require measured results for the final SHA and a
   documented environment and workload. The current null P99 thresholds are
   not evidence of a latency target. Publish calibrated results before citing
   a numeric SLO or comparison.

The next release candidate is `v1.0.0-rc1`, scoped to server and desktop
binaries plus the WIT artifact used by the release pipeline. Language SDK
registry publication (Rust, JavaScript, Python and the Go module tag) is
explicitly excluded. Their `1.0.0` manifests are preparation for GA, not evidence
of a published or qualified GA. `Publish SDKs` ignores prerelease tag pushes
and rejects prerelease manual dispatches; its published-release verifier also
rejects prereleases. Stable releases retain exact manifest-version, CI and
published-inventory gates. SDK dry runs still run for stable tag pushes; RCs
continue to exercise SDK functionality through the main CI SDK tests.

The server's embedded Cargo version and desktop metadata identify RC1.
Before creating that tag, verify the desktop version and any packages being published match the RC
version and their registry's version rules. The inventory check requires the
desktop version to match the tag exactly. The release workflow marks tags with
a prerelease suffix as GitHub prereleases; qualification and publication still
require their own successful runs and authorization.

Windows MSI ProductVersion is numeric and cannot encode `-rc1`. The desktop
configuration therefore uses `1.0.0.0` for MSI while retaining
`1.0.0-rc1` in the application and release asset names. MSI treats the RC and
future `1.0.0` as the same product version for upgrade comparisons; test
upgrades and potential downgrades on a clean Windows machine before release.

Debian prereleases use `~` in the installed package version, so the RC package
is `1.0.0~rc1` and sorts before `1.0.0`; its release asset still carries the
`1.0.0-rc1` name. Rebuilt packages retain Tauri's gzip archive compression.
macOS requires numeric bundle version fields, which are
`1.0.0`; the bundle also records the full RC in `TachyonReleaseVersion`. Check
installation and upgrades on both platforms before release.

To build the desktop installer matrix for a selected branch without publishing
a release, dispatch `integration.yml` with `desktop_qualification=true`. Verify
the run's head SHA and collect its Linux, macOS and Windows artifacts. This
build check does not replace platform installation tests or the required
`push/main` CI and integration runs for the final release SHA.

Local regression checks for the release controls:

```sh
bash tests/scripts/get-tachyon-test.sh
python3 -m unittest discover -s tests/scripts -p 'test_release_gates.py' -v
```

The release inventory checker intentionally fails closed when GitHub does not
return an SHA-256 digest, a matrix asset is missing or unexpected, a checksum
is inconsistent, or the desktop version differs from the tag. Changes to the
release matrix or Tauri asset naming must update the checker and its tests.


## Audit follow-up: 2026-11-01

The following are acceptance criteria, not a record of completed publication.
Record the exact commit, run URLs, downloaded asset hashes and platform results
when each check actually completes. A green PR is not evidence of a release.

| Finding | Repository action / remaining evidence |
| --- | --- |
| TACH-54 | RC1 excludes all language SDK publication; prerelease tag pushes are filtered and manual dispatch plus the published-release gate reject prereleases. Server and desktop versions identify RC1. CI must validate this change. |
| RQ-01 | Requalify the final merged SHA with push/main CI, integration and E2E. On an authorized tag run, verify WIT publication, all 107 expected assets, archive checksums, reference Cosign signature, release manifest and final prerelease flag. No tag run is established by local gate tests. |
| RQ-02 | Install/uninstall the actual published packages on Linux, macOS and Windows; run functional requests and validate Windows RC0→RC1 and RC→GA upgrade/downgrade behavior (numeric MSI version remains unchanged). Commit repeatable platform smoke tests and retain their logs. SDK and third-party WIT consumers remain GA qualification requirements; SDK exclusion does not close these GA requirements. |
| RQ-03 | Keep null P99 thresholds until repeated measurements on representative hardware have raw JSON, workload, hardware, feature flags and exact SHA recorded. Do not infer an SLA from exploratory runs. |
| RQ-04 | Review the always-on ruleset bypass with the repository owner. No ruleset change or risk acceptance is implied by this code change. |
| RQ-05 | Unary gRPC and basic HTTP/3 checks do not prove Retry, 0-RTT, loss handling or long-running QUIC behavior. Qualify those explicitly before promising them in GA. |

External evidence could not be retrieved in the cloud session: Git access works
through the configured proxy with the required execution permission, but GitHub
API requests return `Forbidden`. This prevents verifying Actions, releases,
assets and rulesets, creating a PR through the API, and dispatching platform
qualification workflows. Do not mark those checks passed until access is restored
and their results are inspected.
