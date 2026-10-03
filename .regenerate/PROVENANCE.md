# Provenance

This is the provenance record from Chad Fowler's
[Phoenix Primitives](https://aicoding.leaflet.pub/3mjfruwwuck2d): what produced the
reference implementation, and a ledger of every regeneration since.

Anything marked **unknown** was not written down when it happened, and nothing was
filled in from memory. A gap left visible tells the next reader what a rebuild cannot
reproduce.

## The reference implementation

The reference is a port, not a blind rebuild. The agent that wrote it read the Go
implementation, its tests and the shared `SPEC.md`, and the history before the port
is iroh-acp-go's.

| Item | Value | Source |
| --- | --- | --- |
| Built | 2026-10-02, from iroh-acp-go at commit `e3f5bd8` | `git log` |
| Brief | Port iroh-acp-go to Rust on the official ACP crate, in fewer lines of code | session log, local only |
| Model | Claude Opus 5.5, named in the commit trailers | commit trailers |
| Harness | Claude Code, the same session that maintained the Go implementation | session log, local only |
| Rust toolchain and dependencies | pinned in `Cargo.toml` and `Cargo.lock` | `Cargo.toml`, `Cargo.lock` |
| Format and lint | rustfmt and clippy, config in `rustfmt.toml` and the `[lints]` table | `.github/workflows/lint.yml` |
| Vulnerability scan | cargo-audit, weekly and on every push | `.github/workflows/security.yml` |
| Skill versions, temperature, other model settings | **unknown** | not recorded |

## Changes during the build

The port changed the shared spec. `SPEC.md` gained the Rust inputs in section 2, the
Rust declarations in section 5 and the Rust usage texts in sections 6 and 7.
`DECISIONS.md` gained D21 to D23. tokio, tokio-util and iroh-tickets became direct
dependencies, as iroh and agent-client-protocol leave the runtime, the stream
adapters and the ticket type to them.

Two bugs turned up along the way, and both became lines in the spec:

- An early port served a stream that a rejected peer had opened before `allow`
  returned. noq, the QUIC library under iroh, still yields such a stream. D8 covers
  it, and iroh-acp-go gained a test for it in commit `e3f5bd8`.
- Run against a go-iroh server, the client waited 9 seconds for the server to answer
  its close. The client now waits at most a second. D23 and section 7 cover it.

## Upstream problems found during the build

None in iroh or agent-client-protocol.

## Regeneration ledger

One row per regeneration attempt, including the ones that failed. A failed row is the
most useful kind, because it points at a gap in `SPEC.md`. Rebuilds of the Go
implementation are in iroh-acp-go's ledger.

| Date | Mode | Model and harness | Inputs | Result | Spec gaps found | Spec changes |
| --- | --- | --- | --- | --- | --- | --- |

- **Mode** is `blind` or `guided`, as [`README.md`](README.md) defines them.
- **Inputs** names the commit exported as the reference, the commit of `SPEC.md`,
  `DECISIONS.md` and `PROMPT.md` that the run used, and the dependency pins.
- **Result** says whether the rebuild's own tests and the spec suite passed, names
  any that failed, and gives the line counts from the `size` test.
