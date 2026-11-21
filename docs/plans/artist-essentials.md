# Artist essentials: pens, textures, effects and masks

Tristan's priority (2026-11-21): what makes Harmony worth using for artists
is its **custom pens** (brush and pencil presets, pencil textures, textured
vector brushes) and its **effect nodes with mattes** (Cutter, Tone, Glow and
so on). Symphony needs both, and artists must be able to **bring their
Harmony brushes and pencils with them**.

Sources: the Harmony 27 Premium docs (links below). The facts are summarised
in our own words, and nothing is copied (CLEAN-ROOM.md).

## 1. Brush and pencil presets

What Harmony 27 does:
- Brush, Pencil and Eraser each have a **preset list** in Tool Properties.
  The list menu offers New, Delete, Rename, Import Brushes, Export Brushes,
  and three display modes: small thumbnails, large thumbnails, and stroke
  view (name plus a sample stroke).
- Presets live in the user's **preferences**, not in the scene.
- **Vector brush** presets hold: minimum and maximum size, centreline and
  contour smoothing, tip roundness and angle. A *Textured Vector Brush*
  adds a bitmap tip, paper texture, transparency and a dual tip. Vector
  and bitmap brush presets are separate lists.
- **Pencil** presets hold: minimum and maximum size, centreline smoothing,
  start and end tips (Round, Flat), join (Round, Mitre, Bevel), and
  optionally a **thickness stencil** (a fixed thickness profile along the
  line that ignores pressure) and a **pencil texture**.
- **Export** writes an `.xml` file. **Import** takes Harmony's `.xml` or
  `.tbr`, and Photoshop's `.abr` (Brush and Eraser only).
- Docs: `drawing/about-brush-presets`, `drawing/import-pencil-preset`,
  `drawing/export-brush-preset`,
  `reference/tool-properties/brush-tool-properties`,
  `reference/tool-properties/pencil-tool-properties`.

Symphony plan:
1. ✅ Presets for Brush and Pencil (Eraser presets wait for a sized eraser): list in Tool Properties, New,
   Rename, Delete, and the stroke-view display. Stored in preferences.
2. 🟡 Pencil tips (done) and joins (Round, Flat; Round, Mitre, Bevel) in the
   tessellator.
3. ✅ Thickness stencils (a saved thickness profile).
4. ✅ Export and import of Symphony presets (our own documented format).
5. ⬜ **Import Harmony `.xml` and `.tbr` presets.** Their format isn't
   published. Under CLEAN-ROOM.md we may only learn it from files that
   users created and exported themselves, so we need sample exports
   from someone with a Harmony licence. **Blocked on sample files.**
6. ⬜ Import Photoshop `.abr` tips (Brush and Eraser).

## 2. Textures

- **Pencil textures** live in *pencil texture palettes*, linked by ID like
  colours. Editing a texture swatch updates every line that uses it. The
  texture follows the line's direction and width, so dashes bend with a
  curve and grow with a thicker line. Docs:
  `drawing/about-pencil-line-texture`, `drawing/about-pencil-texture-palettes`.
- **Textured vector brushes** keep vector outlines but fill them with a
  bitmap tip and paper texture. They are re-sampled when the stroke is
  edited. Docs: `drawing/create-textured-brush`.

Symphony plan: ⬜ texture palettes beside colour palettes, ⬜ pencil
texture rendering along the centreline, ⬜ textured vector brush.

## 3. Effect nodes and mattes

How Harmony works:
- Every effect is a **node**. Effects that use a matte take the image
  in one port and a **matte drawing** in the other, and have an **Invert
  Matte** option.
- **Cutter**: cuts the matte's shape out of the image. *Inverted* keeps
  only what the matte covers (the Inverse Cutter).
- **Tone** and **Highlight**: a dark or light region where the image and
  matte overlap, with a blurrable edge (Radial or Directional blur,
  Radius, and Truck Factor).
- **Glow**: a soft light region around the image (same blur parameters).
- **Blur-Gaussian**: Blurriness, Directional (length, width, angle),
  Bidirectional, Repeat Edge Pixels, and an optional matte.
- **Transparency**: 0 to 100, animatable. Also available as layer opacity.
- **Colour-Override**: swaps palette colours, clone palettes or textures
  at render time without changing the palette.
- Docs: `reference/node/combine/cutter-node`, `.../tone-node`,
  `.../highlight-node`, `reference/node/filter/*`.

Symphony plan (render-side first; the Camera view gets a Render Preview
toggle so effects can be seen while working, as in Harmony):
1. ⬜ Render the node graph as a tree of image buffers instead of a flat
   drawing list (`symphony-render`), so effects can sit anywhere.
2. ⬜ Cutter and Inverse Cutter (matte port, Inverted).
3. ⬜ Transparency (animatable).
4. ⬜ Blur-Gaussian, Glow, Tone, Highlight, Shadow.
5. ⬜ Colour-Override, after palette lists exist.
6. ⬜ Node view: add effects from the Node Library and connect matte ports.

## Who does what

- **Claude**: section 1 (presets, tips, joins, stencils), then section 2.
- **Codex**: section 3 (effects engine and nodes). It touches
  `symphony-render`, `symphony-core/src/graph.rs` and the Node view, and
  built the Node Library in #4.
