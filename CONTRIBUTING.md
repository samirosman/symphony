# Contributing to Symphony

Thanks for helping build a free animation app that Harmony users feel at
home in.

## Read this first: clean-room rules
Symphony is built **only** from public documentation, tutorials and
observed behaviour. Read [docs/CLEAN-ROOM.md](docs/CLEAN-ROOM.md) before
contributing. In short:
- Never decompile or disassemble Toon Boom software.
- Never copy Toon Boom code, icons, art, sample files or documentation text.
- Describe behaviour in your own words and link to the public source.

- Build features from the **latest** Harmony version's docs (currently
  27.1), not older ones.

PRs that break these rules will be closed, no matter how good the code is.

## The rules every change follows
- [docs/DESIGN.md](docs/DESIGN.md): how Symphony looks, reads and behaves.
  UI pull requests go through its review checklist (§14).
- [docs/ENGINEERING.md](docs/ENGINEERING.md): how the software works
  (commands and undo, threads, files, profiles, releases).
- Shortcuts and workflows come in two profiles: **Harmony** (as the latest
  Harmony does it) and **Symphony** (our improvements). Improvements go
  in the Symphony profile and never replace the Harmony way (DESIGN §11).

## Getting started
1. Install Rust (stable) from https://rustup.rs
2. `cargo test --workspace`
3. `cargo run --release -p symphony-pentest` to try the pen test

Before opening a PR, run `cargo fmt` and `cargo clippy --workspace --all-targets -- -D warnings`.

## Where help is most wanted
- **Tablets.** Run the pen test and file a *Tablet report* issue. Every
  brand and OS helps.
- **Windows and Linux pen backends** (`crates/symphony-pen`). See
  [docs/PLAN.md §3](docs/PLAN.md#3-tablet-compatibility-plan).
- **Feature research.** Pick an item from [docs/PLAN.md §4](docs/PLAN.md#4-full-harmony-feature-list-target-all-of-it)
  and write up how it behaves, in your own words, with links.

## Code layout
- `symphony-core`: scene model, no UI and no GPU. Keep it that way.
- `symphony-pen`: pen input, one backend module per OS.
- `symphony-pentest`: the pen test window.

## Commits
Write messages that explain *why*, not just *what*. Keep commits focused.
