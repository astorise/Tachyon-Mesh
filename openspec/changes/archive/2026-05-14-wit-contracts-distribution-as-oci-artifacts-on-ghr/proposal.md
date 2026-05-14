# Title: WIT Contracts Distribution as OCI Artifacts on GHCR

## Problem Statement
Currently, the Tachyon-Mesh ecosystem relies on local file copying or Git submodules to share WebAssembly Component Model contracts (`wit/` folder) with consumer projects (like Pulsar or custom FaaS). This creates immediate technical debt: silent ABI drift, opaque dependency chains, and CI integration friction for guest developers.

## Objective
Align Tachyon-Mesh with the Bytecode Alliance standards by distributing its WIT interfaces as versioned OCI artifacts on the GitHub Container Registry (GHCR). 
We will integrate the official `wkg` (Wasm Package Tools) into the existing release pipeline to automatically package and publish the WIT contracts whenever a new GitHub Release (including Release Candidates) is cut.