# Vendored crates

Copies of third-party crates that Symphony changes. Each keeps its own
licence file. Cargo uses them through `[patch.crates-io]` in the root
`Cargo.toml`.

## egui_dock 0.22.0 (MIT, by Adam Gąsior and contributors)

From crates.io. Symphony's changes, marked `// Symphony:` in the code:

- `src/widgets/dock_area/drag_and_drop.rs`: drop-target buttons and the
  areas they highlight have rounded corners, and highlights fade in instead
  of appearing at once (glass-lite look, DESIGN §9).
- `src/widgets/dock_area/show/separator.rs`: dividers snap magnetically
  (10 points) to a divider they continue into, easing in and out of the
  snap. Lined up, they form one junction that drags on both axes. Dividers
  are named "Resize views" for screen readers.
- `src/widgets/dock_area/mod.rs` and `show/window_surface.rs`:
  `DockArea::window_frame` and `DockArea::window_backdrop` let the app give
  floating windows its glass frame and paint the wallpaper backdrop under
  their contents.

To update: copy the new version over this folder, then reapply the
changes marked `// Symphony:`.
- `src/widgets/dock_area/show/leaf.rs`: Close in a tab's right-click
  menu, and middle-click close, depend only on the tab being closeable,
  not on tabs showing a × (Symphony hides it).
- `src/dock_state/tree/mod.rs` and `show/mod.rs`: `Tree::transpose_grid`
  re-stores a 2x2 grid of views the other way round (columns of rows <->
  rows of columns) with the same fractions. `DockArea` calls it when the
  pointer rests on one segment of a long divider where two dividers
  cross and line up, so that segment moves on its own.
