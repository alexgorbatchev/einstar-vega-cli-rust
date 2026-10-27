---
created_on: 2026-10-14 20:10
last_modified: 2026-10-14 22:17
status: current
---

## `vega-cli`

Open the interactive browser using `--ip`, `--port`, and `--output`. Require terminal stdin and stdout. Show file trees, metadata, and a bounded log; skip the project named exactly `Logs` in this browser. Headless listing and downloads include that project when returned by a real device.

File sizes appear when the file pane is at least 35 columns wide. Narrower panes devote their width to names. Resize to reveal more text.

| Key | Action |
| :--- | :--- |
| Up, Down, Home, End, PageUp, PageDown | Move through visible nodes. |
| Space | Toggle selection. |
| Enter, Right, Left | Toggle, expand, or collapse a directory. |
| `d` | Download selected nodes, or the focused node when selection is empty. Ancestor/descendant overlap downloads each file once. One download worker runs at a time. |
| `r`, `x` | Show unsupported rename/delete notices; perform no device writes. |
| `q`, Ctrl-C | Stop workers, await cleanup, and restore the terminal. |

## `vega-cli device`

Command group; require `inspect` or help.

## `vega-cli device inspect`

Handshake and display every device metadata field as `KEY` and `VALUE`. Preserve numeric, binary, tagged, and nested values for display. Require the device address; write no local files.

## `vega-cli project`

Command group; require `list`, `download`, or help.

## `vega-cli project list`

Read projects and print `NAME`, `PATH`, and `DATE TIME`, sorted by exact name. Require the device address; write no local files.

## `vega-cli project download [NAME]`

Download the exact project `NAME`, or every project with `--all`. Complete discovery and path validation before opening the output root. Missing names report available names. Publish files beneath `<output>/<project-name>/<relative-file-path>`; overwrite an existing file only after a verified successful transfer. Continue independent file failures, and return a partial outcome when any fail. Transfers do not resume; a retry downloads each requested file again.

## `vega-cli mock`

Command group; require `serve` or help.

## `vega-cli mock serve`

Bind `0.0.0.0:<port>` and print `mock:0.0.0.0:<port>` before serving. Two synthetic projects contain three 1024-byte zero-filled files each. Validate typed CBOR, project ownership, and byte ranges; cap request bodies at 2 MiB. Ctrl-C or Unix SIGTERM stops the listener, cancels pending request extraction, and awaits connection shutdown. Cooperative shutdown exits 0. No device or output directory is used.

## `vega-cli completion <SHELL>`

Print the selected shell's completion script to stdout. Require a supported `SHELL`; perform no file writes, installation, or network requests.

## `vega-cli skill`

Print the compiled Skills CLI installation pointer verbatim in either mode. Perform no network request and read no repository files. See the publication boundary in `../SKILL.md`.
