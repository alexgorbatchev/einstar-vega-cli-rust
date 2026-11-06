---
created_on: 2026-10-14 14:48
last_modified: 2026-11-06 00:59
status: current
---

# Einstar Vega CLI

Read-only scanner browsing, project downloads, and an offline mock service.

## Commands

- Run: `just run --ip 192.168.30.3`; agent mode: `just run-ai project list --ip 192.168.30.3`.
- Format: `just fmt`; strict verification: `just check`.
- Focused tests: `cargo test --test workflows`; `cargo test --test ui`.
- Build the deliverable: `just build`, producing `bin/vega-cli`.

Use Rust 1.98 or newer. Both packages belong to this Cargo workspace; run root recipes here and package recipes from `crates/einstar-vega`. Compilation requires no external build-slot assignment or migration-workspace lock.

## Build storage

- Reuse the workspace's normal Cargo target and preserve unfinished incremental builds. Resolve target/build directories and configuration overrides before measuring or cleaning; do not create fresh caches, disable incremental compilation, or clean before ordinary builds to reduce reported usage.
- Keep current-task artifacts within 5 GB (5,000,000,000 allocated bytes) across checkouts, worktrees, external build directories, profiles, and target triples. Count temporary files, generated outputs, incremental state, coverage/profiling artifacts, and dedicated caches once; retain the same baseline across retries and handoffs. Exclude pre-existing or other tasks' artifacts, shared caches, downloaded dependencies, installed tools, and runtime data.
- Measure attributable allocations before and after artifact-producing commands and monitor long runs. At the limit, pause only operations that grow counted artifacts, remove verified disposable task-owned outputs, and remeasure. If necessary artifacts still cannot fit, preserve them and report paths, sizes, and the required budget decision; continue independent work. Uncertain ownership never authorizes deletion.
- Retire finished coverage/profile builds once their results are consumed or retained outside disposable storage. Before cleanup, verify exact paths, ownership, tracked contents, symlinks, and active consumers; preserve unfinished normal incremental state and shared storage. Inspect the native cleanup command's scope and verify its exit status and remaining allocations. Do not delete arbitrary hashed Cargo intermediates.

## Contracts

- Reuse the internal `crates/einstar-vega` package in this repository's Cargo workspace for the native CBOR protocol, typed project identities, and atomic capability-based publication. Keep CLI formatting and mock fixtures in the CLI package.
- Keep no-argument execution as the interactive workflow and domain commands subject-first. Global connection/output settings use `VEGA_IP`, `VEGA_PORT`, and `VEGA_OUTPUT`; explicit flags take precedence.
- Keep downloaded files beneath the selected output root, including `.`. Report per-file failures and partial completion truthfully; a published file with failed cleanup remains a partial outcome.
- Keep each file job unique across ancestor and descendant selections. Own one cancellable download worker, observe task failures, and await worker shutdown before restoring the terminal.
- Keep remote identity distinct from decorated labels. Sanitize control characters only for display, and fold home-directory paths through the shared formatter in both output modes.
- Keep rename and delete as unsupported notices; do not introduce device writes.
- The offline mock returns paths belonging to the requested project and validates file/range requests. Its listener follows the source's all-interface binding; it contains synthetic data only.

## Documentation and verification

- Keep `skills/vega-cli/SKILL.md` synchronized with every command, argument, flag, default, environment variable, output contract, exit status, and side-effect change. Update its `last_modified` metadata and verify against the live Clap tree.
- Keep the skill command's offline compiled installation pointer byte-identical in both modes. The local Rust skill and binaries are unpublished; do not imply source Go release archives contain Rust artifacts.
- Change meaningful behavioral tests whenever runtime results change. Require at least 90% line and 90% function coverage for each affected first-party package's non-test source, including unexecuted files and excluding `scripts/`. Root `just coverage` measures the CLI; `just einstar_vega coverage` measures the library. These recipes currently enforce only the line threshold; report missing coverage enforcement separately rather than claiming both metrics pass. Record genuine red/green and compiled implementation-disable/restore evidence.
- Before running a check, compare changed files with its source, tests, fixtures, configuration, lockfiles, toolchain, and compiled, embedded, or validated documentation inputs. Skip and report checks whose inputs are unchanged; run checks with changed or unknown inputs. Never bypass hooks, CI, or other required gates.
- Keep temporary fixtures and verification evidence in this project's `.tmp/`. Use real loopback HTTP, native filesystems, and real PTY checks; hardware and unexecuted platforms remain explicit limitations.
- Preserve MIT licensing and source attribution. Never publish releases, tags, packages, or production deployments without explicit authorization.
- Persist standing rules in place. Keep progress, handoffs, assignments, and task history outside agent instructions.
