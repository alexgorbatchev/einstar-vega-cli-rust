mod einstar_vega 'crates/einstar-vega/justfile'

set dotenv-load := false

default:
    @just --list

run *args:
    cargo run --package einstar-vega-cli --bin vega-cli -- {{ args }}

run-ai *args:
    AGENT=1 cargo run --package einstar-vega-cli --bin vega-cli -- {{ args }}

[private]
_fmt:
    cargo fmt --package einstar-vega-cli

[private]
_fmt-check:
    cargo fmt --package einstar-vega-cli -- --check

[private]
_lint:
    cargo clippy --package einstar-vega-cli --all-targets --locked -- -D warnings

[private]
_test:
    cargo test --package einstar-vega-cli --all-targets --locked

coverage:
    cargo llvm-cov --package einstar-vega-cli --exclude-from-report einstar-vega --all-targets --locked --fail-under-lines 90

docs:
    cargo test --package einstar-vega-cli --doc --locked
    cargo doc --package einstar-vega-cli --no-deps --locked

check: _fmt-check _lint _test docs coverage
    just einstar_vega check

build:
    cargo build --package einstar-vega-cli --release --locked --bin vega-cli
    mkdir -p bin
    cp target/release/vega-cli bin/vega-cli

test: _test
    just einstar_vega test

lint: _lint
    just einstar_vega lint

fmt: _fmt
    just einstar_vega fmt

fmt-check: _fmt-check
    just einstar_vega fmt-check
