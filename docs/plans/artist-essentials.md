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
7. 🟡 **Drag any brush file onto Symphony** (Tristan, 2026-11-21: "just
   drag most brush presets into Symphony and it imports"). Crate
   `symphony-brushes` reads Krita `.kpp`/`.bundle`, MyPaint `.myb`,
   Procreate `.brush`/`.brushset`, Clip Studio `.sut` and GIMP `.gbr` into
   one neutral description, on a background thread. Size, pressure,
   opacity, spacing and smoothing map to Symphony presets. Tip and grain
   images are kept in the brush library for textured brushes (next).
   Still to do: `.abr`, Harmony `.xml`/`.tbr`, Clip Studio tip images and
   pressure curves, and eraser presets.

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
texture rendering along the centreline, ✅ textured vector brush.

Textured vector brush, as built (2026-11-21): a stroke keeps its vector
centreline, pressure and widths plus a `StrokeTexture` (tip, grain, spacing,
opacity, angle, turn with stroke, grain scale). `symphony_vector::stamp`
stamps the tip along the line and multiplies by the grain in drawing space,
fresh for each transform, so moving, scaling or reshaping a line re-samples
the texture. The scene stores the tip and grain images it uses, so it opens
the same on any machine. Export and the canvas use the same stamping. The
canvas stamps the line being drawn a piece at a time, and re-stamps finished
lines on a background thread, showing the last image moved into place
meanwhile (DESIGN §17). Imported brushes become textured vector brushes.
Still to do: Harmony's dual tip, random size/angle/spacing, count, and
bitmap layers.

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

## Later: brush fidelity checkup (parked by Tristan, 2026-11-21)

Tristan tested Krita brushes: they import and work, but don't look like
they do in Krita. The edges are too hard, and pen pressure should also
change softness. He parked this: the Harmony base comes first, and
Harmony's own brushes weren't its strength, so this is a bonus. When we
come back, compare side by side with each app and check:

- **Softness and hardness.** Krita auto brushes have `hfade`/`vfade` (edge
  softness) and a Softness sensor driven by pressure. We read only the
  diameter. Harmony also has a Hardness setting for textured tips.
- **Pressure on opacity and flow.** Krita's Opacity and Flow sensors, and
  Procreate's `dynamicsPressureOpacity`, aren't mapped. Every stamp lands
  at full strength.
- **Flow vs opacity.** We cap the whole stroke at one opacity. Krita and
  Harmony also build up per stamp (flow), which softens overlaps.
- **Tip polarity and greyscale.** Check each app's convention (Krita
  greyscale vs colour tips, ColorAsMask, Procreate white-on-black) against
  its docs or source.
- **Size scale.** Procreate and Clip Studio sizes are best-match guesses.
- **Random size, angle and spacing, scatter, and dual tips.**
- Then Harmony's own Brush Properties tabs (Tip, Transparency, Dual Tip,
  Paper Texture) as the target UI.

### To do (Tristan, 2026-11-23): one Sketch V2 brush better, one worse

After the per-dab texture work (1b35114), Tristan re-imported Sketch V2:
one brush looks better and another looks worse. Next step: get a
side-by-side screenshot of the worse one (Symphony and Krita), find which
setting differs, and compare against Krita 5.0's source (as for the
texture modes). Still unhandled: masking brush, smudge/blending, scatter,
ratio, fuzzy rotation, tilt.

## Who does what

- **Claude**: section 1 (presets, tips, joins, stencils), then section 2.
- **Codex**: section 3 (effects engine and nodes). It touches
  `symphony-render`, `symphony-core/src/graph.rs` and the Node view, and
  built the Node Library in #4.
