---
created_on: 2026-10-14 20:10
last_modified: 2026-10-14 20:10
status: current
---

# Help queries

Use `--help` or `-h` on a command to print its help without device access. Agent help begins with an alert to read `AGENT=1 vega-cli skill`. Generated `help` routes accept a variadic `COMMAND` path; their cloned paths query help and do not add device operations.

- `vega-cli help`
- `vega-cli help device`
- `vega-cli help device inspect`
- `vega-cli help project`
- `vega-cli help project list`
- `vega-cli help project download`
- `vega-cli help mock`
- `vega-cli help mock serve`
- `vega-cli help completion`
- `vega-cli help skill`
- `vega-cli help help`
- `vega-cli device help`
- `vega-cli device help inspect`
- `vega-cli device help help`
- `vega-cli project help`
- `vega-cli project help list`
- `vega-cli project help download`
- `vega-cli project help help`
- `vega-cli mock help`
- `vega-cli mock help serve`
- `vega-cli mock help help`

`vega-cli help device help`, `vega-cli help device help inspect`, `vega-cli help device help help`, `vega-cli help project help`, `vega-cli help project help list`, `vega-cli help project help download`, `vega-cli help project help help`, `vega-cli help mock help`, `vega-cli help mock help serve`, and `vega-cli help mock help help` are generated nested help query paths.
