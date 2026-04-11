## ADDED Requirements

### Requirement: CI validates the renamed desktop wrapper
The CI workflow SHALL validate the renamed `tachyon-ui` desktop wrapper and keep the workspace references aligned with the new client triad layout.

#### Scenario: CI builds the renamed desktop project
- **WHEN** the CI workflow runs on GitHub Actions
- **THEN** it builds `tachyon-ui` in release mode
- **AND** release bundling uses the `tachyon-ui` project path

### Requirement: Release workflow bundles the renamed desktop project
The desktop release workflow SHALL build the Tauri bundles from the `tachyon-ui` project directory on each supported operating system.

#### Scenario: The release workflow targets the renamed desktop directory
- **WHEN** the release workflow invokes the Tauri action
- **THEN** `projectPath` points to `tachyon-ui`
- **AND** frontend dependencies are installed from the `tachyon-ui` directory
