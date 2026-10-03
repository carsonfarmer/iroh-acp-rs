# Regenerate iroh-acp-rs

You are rebuilding a small Rust project from its specification. You have not seen the
existing implementation and you must not look for it. Someone will check your result
against tests you cannot see.

## Your inputs

The workspace holds:

- `SPEC.md`. The behavior the result must have. It is your source of truth. It
  covers a Go and a Rust implementation. You are building the Rust one, so the parts
  marked Rust apply and the parts marked Go do not.
- `DECISIONS.md`. Why the design is the way it is. Use it to choose between options
  when `SPEC.md` leaves room.
- `Cargo.toml` and `Cargo.lock`. They pin the Rust version and the dependencies. Do
  not change the `[package]`, `[dependencies]` or `[lints]` tables, and do not add
  dev-dependencies.
- `LICENSE` and `rustfmt.toml`. Keep them as they are.

## What to produce

1. `src/lib.rs`, the `iroh_acp` library.
2. `src/bin/acp-server.rs` and `src/bin/acp-client.rs`.
3. Your own tests for what you built, in `tests/` (see the tests section). An echo
   agent for them MAY go in `examples/`.

Nothing else. No README and no CI files.

## How to work

1. Read `SPEC.md` and `DECISIONS.md` in full before writing any code.
2. Read the source of the dependencies before you call them. Run `cargo fetch`. The
   sources are then in `$CARGO_HOME/registry/src/` (by default `~/.cargo/registry/src/`),
   in a directory per crate and version as `Cargo.lock` names them. Do not guess
   their APIs from memory. Their `examples/` directories show how to use them. Do not
   copy code from them into the project.
3. Write the smallest program that satisfies `SPEC.md`. The goal is the fewest custom
   lines. The library must stay under 100 non-comment, non-blank lines and each
   binary under 50. The check counts every file of the library or binary. Most of the
   work is already done by iroh and agent-client-protocol. If a function is getting
   long, look in the dependencies for something that does it.
4. Write idiomatic Rust for the edition in `Cargo.toml`. Document public items and the
   reason for anything surprising. Do not comment the obvious.
5. Run `cargo fmt`, `cargo clippy --all-targets` and `cargo build` as you go. Run
   `cargo test -- --include-ignored` before you finish.

## What you must not read

- Any other copy of this project, including one in a parent directory, a sibling
  directory, a cargo cache or a git remote. If you find one, close it and tell me in
  your report.
- The upstream `iroh-acp-rs` and `iroh-acp-go` repositories.
- Any test file that is not one you wrote.

## Tests

Write your own tests from `SPEC.md`. Cover what a reader would want proved: a client
gets an agent through the library, a peer that is not allowed gets nothing and the
server keeps serving, two clients at once, key loading, and the built binaries behaving
the way sections 6 to 8 of the spec describe. Use the loopback address and disabled
relays for tests that need no network, and mark the tests that need the relays
`#[ignore]`. Tests must never touch the real key files, so pass `-key` to the
binaries.

Your tests stay with your code if it is kept. They run with it, and a rebuild whose
own tests fail is not kept. A separate spec suite, which I am not showing you, also
decides whether it is kept:

- It is held out on purpose. If you had it, you could fit the code to the tests and
  stop reading the spec closely. The point of this exercise is to find out whether
  `SPEC.md` is enough to build the project. A rebuild that passes tests it was
  shown proves less.
- The suite compares the `pub` declarations in `src/lib.rs` with the Rust block in
  section 5 of the spec, and your usage errors with the Rust usage texts in sections
  6 and 7. It talks to your library through iroh itself, and runs your binaries with
  the `-key=` and `-allow=` flags. That is why the spec fixes those exactly.
- Your code and your tests are built against the `Cargo.lock` you were given.
- If the suite fails, the spec gets fixed. The tests do not get loosened.

## Where the spec is silent

`SPEC.md` is not perfect. When it says nothing, or two sections disagree, choose the
smallest behavior that fits `DECISIONS.md` and the protocols involved, and write it
down. Do not stop to ask.

## When you finish

Reply with a short report and nothing else:

1. The files you wrote, with the non-comment, non-blank line count of each.
2. The output of `cargo clippy --all-targets` and `cargo test -- --include-ignored`.
3. Every place `SPEC.md` was silent, vague or contradictory, and what you chose.
4. Anything you read outside the workspace and the dependencies.
5. Anything in your code you are unsure about.

Do not commit and do not push.
