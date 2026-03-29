## 1. Managed Guest Sources

- [x] 1.1 Add `guest-csharp/guest-csharp.csproj` and `guest-csharp/Program.cs` so the repository can publish a standalone `guest_csharp.wasm` module that drains stdin and writes `Hello from C# FaaS!`.
- [x] 1.2 Add `guest-java/pom.xml` and `guest-java/src/main/java/com/tachyonmesh/guestjava/Main.java` so TeaVM can emit a standalone `guest_java.wasm` module that drains stdin and writes `Hello from Java FaaS!`.

## 2. Shared Host Compatibility

- [x] 2.1 Add regression tests in `core-host` that lock in the existing hyphenated route-to-module resolution used by `/api/guest-csharp` and `/api/guest-java`.
- [x] 2.2 Raise the default sealed guest fuel budget so managed-language WASI guests can boot while remaining bounded.

## 3. Packaging and Sealed Routes

- [x] 3.1 Extend the `Dockerfile` builder stage to install the .NET 8 WASI workload, `wasi-sdk`, Maven, and OpenJDK 17, compile the C# and Java guests into `/workspace/guest-modules`, and copy them into the runtime image.
- [x] 3.2 Regenerate `integrity.lock` so `/api/guest-csharp` and `/api/guest-java` are sealed alongside the existing routes.

## 4. Verification

- [x] 4.1 Extend `.github/workflows/integration.yml` to assert the deployed responses for `/api/guest-csharp` and `/api/guest-java`.
- [x] 4.2 Verify the change with `cargo test --workspace` and `openspec validate --all`.
