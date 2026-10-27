---
created_on: 2026-10-14 20:10
last_modified: 2026-10-14 22:17
status: current
---

# Configuration

Explicit flags take precedence over their environment variables. There is no configuration file, application cache, account registration, authentication, or telemetry.

| Flag | Type | Default | Environment | Contract |
| :--- | :--- | :--- | :--- | :--- |
| `--ip` | string | empty (`""`) | `VEGA_IP` | Host name, IPv4, or IPv6 address. Required for device, project, and interactive operations; no scheme, path, or embedded port. IPv6 brackets are accepted. |
| `--port`, `-p` | string validated as uint16 | 8080 | `VEGA_PORT` | Integer 1–65535. Scanner connection port, or mock listener port. Validation follows explicit flag precedence. |
| `--output`, `-o` | path | `projects` | `VEGA_OUTPUT` | Trusted local download root, including `.`; flags after subcommands also work. |
| `--version`, `-V` | bool | false | — | Root flag; print raw package version and exit without contacting a device. |
| `--help`, `-h` | bool | false | — | Print the selected help screen without contacting a device. |

`AGENT=1` selects compact text output and agent help. Other values select human output. The interactive root still requires terminal stdin and stdout in either mode.

`vega-cli project download` accepts `--all` (bool, default false), mutually exclusive with positional `NAME`. Exactly one is required. `vega-cli completion` requires `SHELL`: `bash`, `elvish`, `fish`, `powershell`, or `zsh`.

Connections use plaintext HTTP and the specified host and port. Proxies, redirects, automatic retries, and decompression are disabled. Metadata has a 30-second deadline and 16 MiB encoded response limit; interactive connection setup has an additional 10-second limit. File transfers await completion or cancellation without an overall deadline.
