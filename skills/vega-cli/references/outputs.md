---
created_on: 2026-10-14 20:10
last_modified: 2026-10-14 22:22
status: current
---

# Output and recovery

Results and progress go to stdout. Each returned error is printed once on stderr: `ERR: <message>` in agent mode or `[ERROR] <message>` in human mode. Runtime failures do not print usage. Output failures are errors; a failed final summary does not erase publication state.

Human mode uses aligned tables and `[INFO]`, `[OK]`, and `[WARN]` progress. Agent tables use tab-separated columns. Control characters in displayed data are escaped, and home-directory paths use `~`; underlying remote names and local paths retain their identities.

Agent download records:

```text
progress:<destination>:<downloaded>/<total>
published:<destination>
failed:<destination>:<error>
downloaded:<completed> published:<published> failed:<count>
```

Paths and messages can contain colons; these records are compact display text, not a lossless delimiter protocol. `completed` counts fully successful files. `published` also counts files whose final replacement succeeded but staging cleanup failed. `failed` counts independent file failures. A final error after cancellation or output failure retains the previous counts; the ordinary summary may be absent.

| Exit | Meaning |
| :--- | :--- |
| 0 | Success, help, version, completion, skill, or cooperative mock shutdown. |
| 1 | Transport, protocol, filesystem, output, worker, or restoration failure without a classified partial publication. |
| 6 | Independent file failures, or a non-cancellation error after files were published. A combined failure retains a partial outcome from either branch. |
| 64 | Argument/flag validation, missing or invalid device address, or interactive execution without a terminal. |
| 130 | Cancelled operation; earlier files can still be published. |

Downloads stage each file in a private sibling directory, check the declared exact byte length, flush, sync, and explicitly close before atomic replacement. Failed or cancelled transfers preserve existing files. Await shutdown before retrying. Output failure retains independent cleanup/close errors, their typed causes, and file counts. A cleanup error reports a relative staging recovery path; inspect and preserve it until recovery is decided. A published-cleanup error explicitly means the destination was replaced even though cleanup failed. Rename/delete commands are not available.

The chosen output root is trusted local configuration. Descendant directories and files reject symlink traversal on the verified Linux target. Remote POSIX paths reject dot segments and repeated separators. Linux HTTP/filesystem/terminal checks do not prove physical-device or macOS/Windows behavior; Windows reserved filename and alternate-stream semantics remain unverified.
