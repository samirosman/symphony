# Symphony

A free, open-source 2D animation app that feels familiar to animators coming
from Toon Boom Harmony: an xsheet and timeline over one shared exposure, a
peg → drawing → composite node graph, line art and colour art, and paint
by palette ID.

Not affiliated with Toon Boom. Built clean-room from public documentation
(see [docs/CLEAN-ROOM.md](docs/CLEAN-ROOM.md)).

- **[Plan: status, what's left, full feature list](docs/PLAN.md)**
- **[Design rules](docs/DESIGN.md)** and **[engineering rules](docs/ENGINEERING.md)**: every change follows these.
- [Research](docs/RESEARCH.md) · [Language choice](docs/LANGUAGE.md) · [Roadmap](docs/ROADMAP.md) · [Tablets](docs/TABLETS.md)
- [Testing Symphony](docs/TESTERS.md): install the launcher.
- [Contributing](CONTRIBUTING.md): please read the clean-room rules first.

Licensed under GPL-3.0-or-later.

## Build
```sh
cargo test
cargo run --release -p symphony-pentest   # pen test window
```

## Layout
- `crates/symphony-core`: scene model (xsheet, node graph, pegs, evaluation).
  No UI.
- `crates/symphony-pen`: our own pen/tablet input layer (pressure, tilt,
  rotation, eraser), one backend per OS. macOS done; Windows and Linux next.
- `crates/symphony-pentest`: test window that draws with pressure and logs
  every sample to `target/pen-log.csv`.
