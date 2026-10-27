---
created_on: 2026-10-14 14:35
last_modified: 2026-10-14 22:40
status: current
---

# Einstar Vega

Read-only scanner protocol, project selection, and atomic local output library. This internal workspace package belongs to `einstar-vega-cli`; inherit `../../AGENTS.md` and run local recipes from this package directory.

## Commands

- Format: `just fmt`; verify: `just fmt-check`.
- Check: `just check` (format, strict Clippy, behavioral tests, docs, and 90% line coverage).
- Focused tests: `cargo test --test protocol`, `cargo test --test publication`, `cargo test --test tree`.
- Build: `just build`.

Use the migration parent's coordinated build slot and `flock` for compilation. Preserve unfinished normal incremental caches and the combined storage budget in `../../../AGENTS.md`.

## Contracts

- Use native CBOR values for device metadata and typed request structs. Do not coerce binary values into JSON text or infer the undocumented first list-result item's semantics.
- Keep file-info requests as raw path bytes and sizes as exact eight-byte big-endian integers. Downloads must reject both truncated and oversized payloads.
- Do not enable HTTP redirects, automatic retries, proxies, or decompression. Keep cancellation effective during pending response and output writes.
- Keep remote POSIX paths distinct from native output paths. Validate before normalization; URL parsing elides explicit default ports.
- Keep tree IDs scoped to their owning tree and selections deduplicated. Never recover identity from display labels.
- Retain capability directory handles and reject symlink descendants. Publish only after verified transfer, flush, sync, and fallible close. A cleanup failure after rename must remain `Error::Published`; do not falsely promise rollback.
- Normal cancellation awaits cleanup. Dropped futures provide only best-effort cleanup; preserve recovery paths in explicit cleanup errors.

## Boundaries

- Always change behavioral tests alongside runtime results and maintain at least 90% line coverage; `scripts/` is excluded. Record genuine failing regressions and temporary implementation-disable/restore evidence.
- Keep tests offline using real loopback HTTP servers and temporary native filesystems. Do not claim physical-device verification from those tests.
- Preserve source licensing and attribution. Never publish releases, tags, packages, or production deployments without explicit authorization.
- Persist standing project rules in place; keep progress, handoffs, and task history outside agent instructions.
