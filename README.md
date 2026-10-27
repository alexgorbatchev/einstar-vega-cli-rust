`vega-cli` browses and downloads Einstar Vega scanner projects for people who need direct access without account registration. It provides an interactive browser, headless commands, and an offline scanner simulation.

# What It Does

- **Browse projects**: Inspect device metadata and expand project file trees.
- **Download safely**: Select files or complete projects while preserving existing files on failed transfers.
- **Automate workflows**: List and download projects with compact `AGENT=1` output.
- **Work offline**: Run a synthetic scanner service for local integration checks.

# How It Works

- Give `vega-cli` the scanner's address with `--ip` and optionally `--port`.
- Open the interactive browser, or list projects and download an exact project name.
- Files land under `projects/<project-name>/` by default; choose another root with `--output`.
- Stop the browser with `q` or Ctrl-C; pending workers finish cancellation and cleanup before the terminal is restored.

# How it Really Works

- Requests use the scanner's plaintext HTTP interface without authentication. Redirects, proxies, automatic retries, and decompression are disabled. Metadata requests have a 30-second deadline and 16 MiB response limit; file transfers remain cancellable without an overall deadline.
- Each file is staged beside its destination. Exact length, flush, sync, and close checks precede atomic replacement. Failed transfers preserve existing bytes. Earlier completed files remain when another file fails; retries download requested files again.
- Selection uses file identities. Selecting both a directory and its child downloads each file once; one interactive download runs at a time. The browser skips the `Logs` project, while headless operations include every returned project.
- Progress and results go to stdout; a returned error is printed once on stderr. `AGENT=1` selects tab-separated tables and compact progress; human mode uses aligned tables and status lines. Displayed control characters are escaped and home paths use `~`.
- Exit 0 means success; 64 means invalid input; 130 means cancellation. Exit 6 signals file failures or a non-cancellation error after publication; exit 1 covers other failures. Cancellation can leave earlier files published. Cleanup errors preserve staging recovery paths and distinguish already-published destinations.
- The mock binds all interfaces on the selected port and serves synthetic zero-filled files. Ctrl-C and Unix SIGTERM stop requests and release the listener. Rename and delete show unsupported notices and make no device writes.

# Prerequisites

- An [Einstar Vega](https://www.einstar.com/) reachable over its HTTP interface, normally port 8080, for device operations.
- Terminal stdin and stdout for interactive browsing. Headless commands and the mock work without a terminal.

# Installation

Rust release artifacts are unpublished. The local deliverable is `bin/vega-cli`; contributor build instructions are in [AGENTS.md](AGENTS.md). The canonical source repository's Go archives do not contain this Rust binary.

# Quick Start

```sh
vega-cli --ip 192.168.30.3
AGENT=1 vega-cli project list --ip 192.168.30.3
vega-cli project download 'Project1' --ip 192.168.30.3 --output ./scans
vega-cli project download --all --ip 192.168.30.3
```

For the offline simulation, run `vega-cli mock serve --port 8080` in one terminal, then:

```sh
AGENT=1 vega-cli project list --ip 127.0.0.1 --port 8080
```

Sample Output (tab-separated):

```text
NAME	PATH	DATE TIME
Project1	/mnt/sdcard/Project1	2023-10-27T10:00:00Z
Project2	/mnt/sdcard/Project2	2023-10-28T11:00:00Z
```

# Options & Flags

| Flag | Short | Default | Description |
| :--- | :--- | :--- | :--- |
| `--ip <host>` | — | Empty | Device address; `VEGA_IP` supplies the default. |
| `--port <port>` | `-p` | `8080` | Scanner or mock listener port; `VEGA_PORT` supplies the default. |
| `--output <path>` | `-o` | `projects` | Download root, including `.`; `VEGA_OUTPUT` supplies the default. |
| `--version` | `-V` | `false` | Print the raw version and exit. |
| `--help` | `-h` | `false` | Print help. |

`vega-cli project download [NAME]`:

| Flag | Short | Default | Description |
| :--- | :--- | :--- | :--- |
| `--all` | — | `false` | Download all projects instead of one exact `NAME`. |

# Interactive Controls

Use arrows, Home, End, PageUp, and PageDown to navigate; Enter toggles directories, Right expands, and Left collapses. Space toggles selection, `d` downloads selected or focused files, and `q` quits. `r` and `x` report unsupported rename and delete operations.

At narrow widths the browser hides the size column to keep file names visible. Resize the terminal to show more of long names and device metadata.

# Agent Usage

Run `AGENT=1 vega-cli skill` for the offline Skills CLI installation pointer. The local operational guide is [skills/vega-cli/SKILL.md](skills/vega-cli/SKILL.md); its publication is pending.

# Compatibility

Local HTTP, filesystem, process, and terminal verification targets Linux. Physical-device behavior and macOS/Windows execution remain unverified; Windows reserved filenames and alternate data streams have not been validated. The source reports firmware `v1.3.0-22` testing, which is not a Rust hardware verification claim.

Protocol behavior follows [alexgorbatchev/einstar-vega-cli](https://github.com/alexgorbatchev/einstar-vega-cli), which credits [rabits/einstar-vega-extract](https://github.com/rabits/einstar-vega-extract).

# License

[MIT License](LICENSE), copyright 2026 Alex Gorbatchev.
