# AGENTS.md

Instructions for coding agents working on iroh-acp-rs. [README.md](README.md) covers
what the project does and how to use it.

## The project

A Rust library and two binaries that carry the Agent Client Protocol over iroh, so an
editor on one machine can use an agent on another. Nearly all the work is done by
[iroh](https://github.com/n0-computer/iroh) and
[agent-client-protocol](https://github.com/agentclientprotocol/rust-sdk). This
repository holds the glue.

| Where | What it is |
| --- | --- |
| `src/lib.rs` | The library, crate `iroh_acp`. |
| `src/bin/acp-server.rs`, `src/bin/acp-client.rs` | The two binaries. |
| `tests/irohacp.rs` | The tests, for the library and for the built binaries. |
| `examples/echo.rs` | The echo agent the tests run. |
| `.github/` | CI for tests, clippy, rustfmt and cargo-audit, and the Dependabot config. |

## Commands

```bash
cargo build
cargo clippy --all-targets -- -D warnings
cargo test                          # loopback only, no network
cargo test -- --include-ignored     # the full suite, including the n0 relays
cargo fmt                           # the config is in rustfmt.toml
```

Run `cargo fmt`, the full suite and clippy before you call a change done.

## Plan before you build

For a new feature or any change in behavior, use the
[grill-me](https://github.com/mattpocock/skills/tree/main/skills/productivity/grill-me)
skill to interview the user before you write code.

## Writing Rust

Write idiomatic Rust for the edition in `Cargo.toml`. The `[lints]` table and
`rustfmt.toml` set the rest.

## Rules

- **Stay inside the size budget.** The library stays under 100 lines of code and each
  binary under 50, counting lines that are neither blank nor comments in all of a
  target's files. CI fails when one goes over. When code grows, look in iroh or
  agent-client-protocol for something that already does the job.
- **Read the dependency source before you call it.** `cargo fetch` puts it under
  `~/.cargo/registry/src/`, in a directory per crate and version. Do not guess an API
  from memory.
- **Treat the interface as a contract.** Other people's clients and servers depend
  on the `pub` signatures, the ALPN `acp/1`, the flags, stdout and the log lines.
  CI checks them, and they change only when the user asks.
- **Keep versions in one place.** Rust, crate and tool versions live in `Cargo.toml`,
  `Cargo.lock` and the workflow files. Do not repeat them in docs, comments or tests.
- **Ask before adding a direct dependency.** That includes dev-dependencies.
- **Keep the docs in step.** A change in behavior updates the code, its tests and
  `README.md` together.

## Tests

- `tests/irohacp.rs` uses only the public API and the built binaries. A test module
  inside `src/` counts against the size budget.
- Never loosen a test to make code pass. If a test is wrong, tell the user before you
  change it.
- Tests use a temporary directory and pass `-key` to the binaries. They must never
  read or write the real key files in the user's config directory.
- The full suite dials through the n0 public relays, so it needs the network. The
  tests that need the relays are marked `#[ignore]`, so plain `cargo test` skips them.

## Upstream bugs

Report and fix a bug in iroh or agent-client-protocol upstream. Carry a workaround
here only until the fix is released, and remove it in the commit that updates the
dependency.

## Commits

- Keep commits small. The subject is a short imperative sentence, and the body says
  why.
- Never bypass git hooks with `--no-verify`.
- Key files are credentials. `*.key` is ignored, so never force one in or print its
  contents.
- Do not push, tag or open a pull request unless the user asks.

## Writing

Docs and comments use plain, direct prose: short sentences, no em dashes and no hype.
Do not name the maintainer in the docs.
