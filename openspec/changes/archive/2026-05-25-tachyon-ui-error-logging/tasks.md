## 1. Desktop Configuration And Writer

- [x] 1.1 Add `tachyon-ui/src/app_config.rs` with a versioned, generated JSON configuration and logging defaults for level, relative file path, rotation size, and retention count.
- [x] 1.2 Validate that configured log paths remain within the local application-data directory and that rotation bounds are acceptable.
- [x] 1.3 Add `tachyon-ui/src/app_logging.rs` with JSON Lines output, severity filtering, message/source bounds, and numbered file rotation.
- [x] 1.4 Add Rust unit tests covering default configuration creation, path traversal rejection, level filtering, and file rotation.

## 2. Tauri And Frontend Error Capture

- [x] 2.1 Initialize configuration and logging during Tauri setup, expose `log_frontend_event`, and record fatal runtime errors after logger startup.
- [x] 2.2 Add `tachyon-ui/src/utils/appLogger.ts` to submit redacted frontend log events and capture global errors and unhandled promise rejections.
- [x] 2.3 Record failures caught by the shared network workflow, including failed IPC commands, apply-and-seal execution, and bounded reconnect probes.
- [x] 2.4 Route production Tauri call sites outside the network workflow through the logging boundary, including MFA, topology, and scope-controller flows.
- [x] 2.5 Record mesh event-listener registration failures from the frontend bootstrap path.

## 3. Secret Safety And Operator Documentation

- [x] 3.1 Ensure frontend logging events omit invocation arguments that can include credentials, tokens, TOTP values, certificates, or configuration payloads.
- [x] 3.2 Add a frontend unit test proving that a failed credential-bearing invocation logs only its source and error message.
- [x] 3.3 Document the generated configuration file location, JSON schema, accepted log levels, rotation behavior, and restart requirement in `README.md`.

## 4. Verification

- [x] 4.1 Run `cargo test -p tachyon-ui` and verify all Rust configuration and logging unit tests pass.
- [x] 4.2 Run `npm test` in `tachyon-ui` and verify the frontend suite passes, including the secret-exclusion logging test.
- [x] 4.3 Run `npm run build` in `tachyon-ui` and verify the TypeScript and Vite production build succeeds.
- [x] 4.4 Run `cargo fmt --check --package tachyon-ui` and `git diff --check` to verify formatting and patch hygiene.
