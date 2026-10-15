# Contributing to mira

Thanks for helping. Bug reports, ideas and pull requests are all welcome.

## Before you start

- **Bugs**: open an [issue](https://github.com/IGUNUBLUE/mira/issues/new/choose) with steps to
  reproduce, what you expected, and your distro, compositor and GPU driver.
- **Features**: open an issue first for anything bigger than a small fix, so we can agree on
  the approach before you spend time on it.
- **Security**: don't open a public issue; see [SECURITY.md](SECURITY.md).

mira follows a few product rules (read them in [AGENTS.md](AGENTS.md#product-rules)): terminal
first, light on memory, no telemetry, integrate tools instead of rebuilding them, and never
patch dependencies. Pull requests that go against them are unlikely to be merged.

## Development

You need Rust (stable, via [rustup](https://rustup.rs)), `git`, and on Debian/Ubuntu:

```sh
sudo apt install pkg-config libxkbcommon-dev libwayland-dev libfontconfig1-dev
```

Then:

```sh
cargo run -- /path/to/some/repo
```

To try mira without touching your own settings, agent history or herdr session, point it at a
temporary config and state directory and skip herdr:

```sh
T=$(mktemp -d); mkdir -p "$T/cfg/mira"; echo 'terminal = "shell"' > "$T/cfg/mira/config"
XDG_CONFIG_HOME="$T/cfg" XDG_STATE_HOME="$T/state" cargo run -- /path/to/repo
```

Never stop or restart your real herdr server from inside one of its panes while testing; see
the herdr notes in [AGENTS.md](AGENTS.md#gotchas).

## Quality gate

CI runs exactly this; please run it before opening a pull request:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

- No `unsafe`, no `unwrap`/`expect` outside tests, no new dependency without a reason
  (memory, binary size, compile time). Pin exact versions where the project does.
- Put pure logic in functions without IO and test it there.
- Keep `view` cheap: no IO, no per-row formatting; compute in `update`.
- Colors only through `theme::p()`.

## Commits and pull requests

- [Conventional Commits](https://www.conventionalcommits.org): `feat(scope): …`, `fix: …`,
  `docs: …`. Say *why* in the body, not only what.
- One topic per pull request. Describe what changed, how you tested it, and include a
  screenshot for UI changes.
- Update the docs (`README.md`, `docs/`) when behavior or settings change, and add an entry
  under *Unreleased* in [CHANGELOG.md](CHANGELOG.md).

By contributing you agree that your work is licensed under the project's dual
MIT / Apache-2.0 license.

## Using coding agents

mira is built with coding agents, and [AGENTS.md](AGENTS.md) is written for them as much as for
people. Skills in `.agents/skills/` give agents mira-specific knowledge; third-party skills are
pinned in `skills-lock.json` and restored with `npx skills experimental_install`. You are
responsible for what you submit, whoever typed it.

## Code of conduct

This project follows the [Code of Conduct](CODE_OF_CONDUCT.md).
