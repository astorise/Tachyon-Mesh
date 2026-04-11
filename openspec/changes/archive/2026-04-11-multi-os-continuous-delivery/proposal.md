# Proposal: Change 060 - Multi-OS Continuous Delivery for Tauri CLI

## Context
The Tachyon Mesh project now includes a graphical interface and CLI built with Tauri (`tachyon-cli`). Currently, the CI pipeline (`ci.yml`, `integration.yml`) only tests the Rust code on Linux Ubuntu. However, building native desktop applications requires the specific host operating system: macOS requires Darwin/Xcode to build `.dmg` files, and Windows requires MSVC to build `.msi` or `.exe` files. We cannot cross-compile GUI apps reliably from a single Linux container.

## Objective
1. Implement a Continuous Delivery (CD) pipeline using GitHub Actions specifically for the `tachyon-cli` project.
2. Utilize a "Build Matrix" strategy to concurrently spawn macOS, Windows, and Linux runners.
3. Automatically build, package, and publish the compiled binaries (AppImage, DMG, MSI) to a GitHub Release draft whenever a new version tag is pushed.

## Scope
- Create `.github/workflows/release.yml`.
- Ensure Node.js (for the frontend) and Rust (for the backend) are properly provisioned on all runners.
- Update `tachyon-cli/tauri.conf.json` if necessary to ensure all bundle targets are enabled.

## Success Metrics
- Pushing a tag like `v1.0.0` triggers the workflow.
- The workflow completes successfully across all 3 operating systems.
- A GitHub Release draft is created containing `tachyon-studio_1.0.0_amd64.AppImage`, `tachyon-studio_1.0.0_aarch64.dmg`, and `tachyon-studio_1.0.0_x64.msi`.