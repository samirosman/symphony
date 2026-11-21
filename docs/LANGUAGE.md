# Language choice: Rust (confirmed, with one risk to test first)

## Verdict
Rust is a good fit. Use **Rust for the engine and the app**, render with
**wgpu**, and keep the engine free of UI code so the front end can change.

## Why Rust
- **Safety**: an animation app holds hours of unsaved work. Memory bugs
  cause crashes, and crashes lose work. Rust rules out a whole class of
  them, a common complaint against C++ tools.
- **Performance**: same tier as C++. That matters for vector tessellation,
  deformers and playback.
- **Tooling**: cargo, tests and cross-platform builds work out of the box.
- **Precedent**: Graphite is a node-based 2D editor in Rust on wgpu.
  `vello`, `lyon`, `kurbo` and `tiny-skia` cover vector rendering.

## The honest alternative: C++ with Qt
Harmony, OpenToonz and Krita are all C++/Qt. Qt gives mature docking, tablet
input and native widgets today. If the goal were "ship fastest", C++/Qt would
be the safer bet. It loses on safety, build pain and contributor appeal.

## Biggest risk: pen tablet pressure and tilt
Drawing is the core of the app. Before building UI, prototype pressure and
tilt input on macOS (Apple Pencil via Sidecar, Wacom), Windows Ink and Linux.
- If winit's pen support works well enough → pure Rust (eframe/egui or a
  custom wgpu UI).
- If not → native pen backends behind a small trait, or a Qt front end via
  cxx-qt with the Rust engine unchanged. This is why the engine stays
  UI-agnostic.

## UI layer, starting point
`egui` + `egui_dock` for the first window. It's fast to iterate and gives
dockable views right away. Expect to replace hot views (Camera canvas,
Timeline, Xsheet) with custom wgpu widgets for performance and Harmony-like
look.

## Decision (2026-11-21)
We're going with Rust and writing our own pen layer (`symphony-pen`), so pen
input doesn't depend on the UI toolkit. Goal: every tablet. macOS reads the
standard AppKit tablet events that Wacom, Veikk, XP-Pen, Huion and Sidecar
drivers all send. Windows will need both Windows Ink and Wintab.
