# PRD: Docker Build Optimization

## Introduction

The current Docker build and publish workflow takes over an hour to complete, which severely impacts developer productivity and release velocity. This PRD outlines a comprehensive optimization strategy to reduce build times to under 10 minutes using industry best practices for Rust container builds.

## Goals

- Reduce Docker build time from 1+ hour to under 10 minutes (6x+ improvement)
- Maintain multi-architecture support (linux/amd64 and linux/arm64)
- Preserve image size optimization (slim runtime image)
- Enable incremental builds where only changed code is recompiled
- Minimize CI costs through efficient caching

## Background Research

Based on research from [Depot.dev](https://depot.dev/docs/container-builds/how-to-guides/optimal-dockerfiles/rust-dockerfile), [Luca Palmieri's blog](https://www.lpalmieri.com/posts/fast-rust-docker-builds/), and [vladkens' Medium article](https://medium.com/@vladkens/fast-multi-arch-docker-build-for-rust-projects-a7db42f3adde):

- **cargo-chef** provides 5x speedup by caching dependency compilation separately from source
- **sccache** provides additional 75%+ reduction through compiler-level caching
- **Cross-compilation** is 40x faster than emulation for multi-arch builds
- **Parallel builds** can cut multi-arch time in half

## User Stories

### US-001: Add cargo-chef for dependency caching
**Description:** As a CI system, I want to cache compiled Rust dependencies separately from source code so that dependency changes don't require full rebuilds.

**Acceptance Criteria:**
- [ ] Add cargo-chef to Dockerfile with planner, cacher, and builder stages
- [ ] Recipe file generated from Cargo.toml and Cargo.lock
- [ ] Dependencies cached in separate Docker layer
- [ ] Source code changes don't invalidate dependency cache
- [ ] Typecheck passes
- [ ] Docker build succeeds locally

### US-002: Add sccache for compiler-level caching
**Description:** As a CI system, I want fine-grained compiler caching so that unchanged compilation units are reused across builds.

**Acceptance Criteria:**
- [ ] Install sccache in builder stage
- [ ] Configure RUSTC_WRAPPER=sccache
- [ ] Add BuildKit cache mount for sccache directory
- [ ] Cache persists across builds via GHA cache or external storage
- [ ] Typecheck passes
- [ ] Docker build succeeds locally

### US-003: Add BuildKit cache mounts for cargo registry
**Description:** As a CI system, I want to cache the cargo registry and git dependencies so that crate downloads are not repeated.

**Acceptance Criteria:**
- [ ] Add cache mount for /usr/local/cargo/registry
- [ ] Add cache mount for /usr/local/cargo/git
- [ ] Use sharing=locked to prevent concurrent access issues
- [ ] Cache mount syntax works with BuildKit
- [ ] Typecheck passes
- [ ] Docker build succeeds locally

### US-004: Implement cross-compilation with cargo-zigbuild
**Description:** As a CI system, I want to cross-compile for both architectures natively instead of using slow QEMU emulation.

**Acceptance Criteria:**
- [ ] Install cargo-zigbuild and zig in builder stage
- [ ] Configure cross-compilation targets (x86_64-unknown-linux-musl, aarch64-unknown-linux-musl)
- [ ] Build both architectures in single cargo command
- [ ] Copy correct binary based on TARGETPLATFORM ARG
- [ ] Emulation (QEMU) no longer required
- [ ] Typecheck passes
- [ ] Multi-arch Docker build succeeds

### US-005: Update GitHub Actions workflow for optimized builds
**Description:** As a maintainer, I want the CI workflow to leverage all caching optimizations for fastest possible builds.

**Acceptance Criteria:**
- [ ] Enable BuildKit with cache mounts in workflow
- [ ] Configure sccache with GHA cache backend
- [ ] Remove QEMU setup (no longer needed for cross-compilation)
- [ ] Add build timing output for monitoring
- [ ] Workflow passes on push to main
- [ ] Build time reduced by at least 50%

### US-006: Add parallel architecture builds (optional optimization)
**Description:** As a CI system, I want to build each architecture in parallel jobs then combine them, for cases where cross-compilation isn't suitable.

**Acceptance Criteria:**
- [ ] Create separate jobs for amd64 and arm64 builds
- [ ] Add manifest creation job that depends on both
- [ ] Use docker manifest create to combine images
- [ ] Total build time is roughly max(amd64, arm64) instead of sum
- [ ] Workflow passes on push to main

### US-007: Document build optimization and benchmarks
**Description:** As a developer, I want documentation explaining the build optimizations and expected performance.

**Acceptance Criteria:**
- [ ] Update docs/guides/docker-mcp-setup.md with build info
- [ ] Document local build commands with caching
- [ ] Include benchmark results (before/after times)
- [ ] Explain cache invalidation scenarios
- [ ] All markdown links valid

## Functional Requirements

- FR-1: Dockerfile must use multi-stage build with cargo-chef stages (planner, cacher, builder, runtime)
- FR-2: All compilation must use sccache wrapper (RUSTC_WRAPPER=sccache)
- FR-3: BuildKit cache mounts must be used for cargo registry, git deps, and sccache
- FR-4: Cross-compilation must target both x86_64 and aarch64 musl
- FR-5: Final runtime image must remain minimal (debian-slim or alpine based)
- FR-6: Build args (VERSION, COMMIT_SHA) must still be supported
- FR-7: Image labels for MCP compatibility must be preserved

## Non-Goals

- Switching to a paid CI service (Depot.dev, Blacksmith) - keep using free GitHub Actions
- Reducing the number of supported architectures
- Changing the MCP server binary or runtime behavior
- Adding new features to the application itself

## Technical Considerations

- cargo-chef requires same Rust version across all stages
- sccache GHA cache has 10GB limit - may need S3 backend for large projects
- musl builds may have different behavior than glibc - need testing
- cargo-zigbuild requires zig toolchain installation
- Cache mounts require `# syntax=docker/dockerfile:1` directive

## Success Metrics

- Docker build time < 10 minutes (down from 60+ minutes)
- Incremental builds (source-only changes) < 5 minutes
- Multi-arch support maintained
- Image size remains under 100MB
- All existing tests pass

## Open Questions

- Should we use musl (fully static) or continue with glibc?
- Is GitHub Actions cache sufficient or do we need S3-backed sccache?
- Should we split into parallel jobs or use single cross-compile approach?
