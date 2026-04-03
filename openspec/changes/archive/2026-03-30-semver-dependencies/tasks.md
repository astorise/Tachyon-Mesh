# Tasks: Change 025 Implementation

- [x] 1.1 Add `semver = "1.0"` to both `tachyon-cli` and `core-host`.
- [x] 1.2 Extend sealed route metadata with `name`, `version`, and `dependencies`, while keeping older manifests compatible through defaults.
- [x] 1.3 Teach `tachyon-cli generate` to accept explicit route SemVer metadata via `--route-name`, `--route-version`, and `--route-dependency`.
- [x] 1.4 Build a dependency registry in `core-host` at startup and fail boot when no compatible dependency version is loaded.
- [x] 1.5 Resolve internal mesh aliases like `http://tachyon/<service>` to the highest compatible sealed route version declared by the caller.
- [x] 1.6 Cover the SemVer route metadata, startup validation, and mesh-resolution behavior with Rust tests.
- [x] 1.7 Convert this change to valid OpenSpec delta specs and verify with `openspec validate --all`.
