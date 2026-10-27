---
created_on: 2026-10-14 14:48
last_modified: 2026-10-14 22:40
status: current
---

# Einstar Vega CLI

Read-only scanner browsing, project downloads, and an offline mock service.

## Commands

- Run: `just run --ip 192.168.30.3`; agent mode: `just run-ai project list --ip 192.168.30.3`.
- Format: `just fmt`; strict verification: `just check`.
- Focused tests: `cargo test --test workflows`; `cargo test --test ui`.
- Build the deliverable: `just build`, producing `bin/vega-cli`.

Use Rust 1.98 or newer and the parent's coordinated build slots and storage policy. Keep unfinished normal incremental caches. Run compilation inside the assigned shared `flock`; see `../AGENTS.md`.

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
- Change behavioral tests whenever runtime results change, and maintain at least 90% line coverage; `scripts/` is excluded. Record genuine red/green and compiled implementation-disable/restore evidence.
- Keep temporary fixtures and verification evidence in this project's `.tmp/`. Use real loopback HTTP, native filesystems, and real PTY checks; hardware and unexecuted platforms remain explicit limitations.
- Preserve MIT licensing and source attribution. Never publish releases, tags, packages, or production deployments without explicit authorization.
- Persist standing rules in place. Keep progress, handoffs, assignments, and task history outside agent instructions.
