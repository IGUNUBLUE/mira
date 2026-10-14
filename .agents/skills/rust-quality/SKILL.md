---
name: rust-quality
description: Quality gate and Rust conventions for mira. Use before finishing any code change in mira, when reviewing code, fixing clippy warnings, adding tests, handling errors, adding dependencies, or improving performance or memory use.
---

# mira quality gate

General Rust idioms (ownership, errors, clippy, tests, docs): see the `rust-best-practices` skill. Bugs and test failures: follow `systematic-debugging` (root cause before fixes). This file holds what is specific to mira and wins on conflicts (e.g. no `anyhow`/`thiserror`, no new deps without a reason).

## Gate (all must pass before reporting a change as done)
1. `cargo fmt --check` (rustfmt.toml: max_width 120)
2. `cargo clippy --all-targets -- -D warnings`
3. `cargo test`
4. `cargo build --release` when touching main.rs, Cargo.toml or features
5. `cargo install --path . --locked` only when the user wants to try it; the running mira is replaced only on restart

## Rules
- No `unwrap()`/`expect()` outside tests unless provably infallible (comment why). Release uses `panic = "abort"`: a panic kills the user's agents' terminal.
- Errors reach the user: map to a `Msg` and show them in the status bar. Never drop a `Result` with `let _ =` unless failure is genuinely irrelevant (comment it).
- Newtypes for ids that can be confused (pane ids, tree ids); stable ids instead of `Vec` indices for things that can be removed.
- Pure logic (state machines, parsers, budgets) lives in functions without IO so it can be unit-tested; IO stays at the edges.
- New dependency only with a reason (memory, binary size, compile time). Pin exact versions (`=x.y.z`). Prefer std. Check `cargo tree -d` for duplicates.
- No telemetry. The only network access is commit-message generation on click (`openai.rs`, through `curl`, secrets on its stdin); nothing else may talk to the network.
- Never interpolate untrusted values into a shell; pass argv arrays or JSON.
- Conventional Commits; no AI attribution trailers.

## Tests
- Unit tests next to the code in `#[cfg(test)] mod tests`.
- git: integration tests in a temp repo; snapshots must never touch the user's index.
- Temp dirs: unique per test (atomic counter), not just `process::id()`.
- Tests that mutate env vars race with each other: inject paths instead where possible.
- Bug fix = a regression test that fails before the fix.
- Protocol clients (herdr): decode recorded JSON fixtures, including unknown extra fields and error lines.

## Lints
`Cargo.toml [lints]`: `unsafe_code = "forbid"`, clippy `all`, `unwrap_used`/`expect_used` (allowed in tests via
clippy.toml), `dbg_macro`, `todo`. CI (`.github/workflows/ci.yml`) runs fmt, clippy `-D warnings` and tests.
