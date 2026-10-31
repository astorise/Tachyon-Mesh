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

The intended first release candidate is `v1.0.0-rc0`. Before creating that
tag, verify the desktop version and any packages being published match the RC
version and their registry's version rules. The inventory check requires the
desktop version to match the tag exactly. The release workflow marks tags with
a prerelease suffix as GitHub prereleases; qualification and publication still
require their own successful runs and authorization.

Windows MSI ProductVersion is numeric and cannot encode `-rc0`. The desktop
configuration therefore uses `1.0.0.0` for MSI while retaining
`1.0.0-rc0` in the application and release asset names. MSI treats the RC and
future `1.0.0` as the same product version for upgrade comparisons; test
upgrades and potential downgrades on a clean Windows machine before release.

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
