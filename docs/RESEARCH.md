# Harmony research notes

Built from Toon Boom's **public** docs (docs.toonboom.com, learn.toonboom.com)
and public talks and tutorials. Items marked *(verify)* still need checking
against the docs or a tutorial video.

## Target version
**Harmony 27.1 (2026).** Always implement from the newest docs. The notes
below were first gathered from versions 20–25 and must be re-checked
against 27 before each feature is built.

New since those versions:
- **Harmony 27**: Attribute Controller, a non-destructive rig control.
  It adds custom offsets to rig attributes, has a Setup Mode for
  training poses, can learn from existing keyframes, and partly replaces
  Master Controllers. Also a Reference view Mirror mode and a Selection
  Lock node, which makes rig layers unselectable in the Camera view.
- **Harmony 27.1**: Skeleton Controllers (beta), which let you animate
  once and reuse it on any character with the same skeleton. Also pose
  indicators for Attribute Controllers, and a detail view for the
  Library audio bin.
- **Harmony 25.2**: Art Layer Manager, and an AI Generative Fill operation.
- **Harmony 25**: a compact scene format that packs drawings and palettes
  into one file.

## What animators expect to feel the same

### 1. Two timing views of one scene
- **Timeline**: layers listed top to bottom, frames running left to right.
  The layers are the same thing as Xsheet columns.
- **Xsheet**: the traditional exposure sheet. Columns are elements, rows are
  frames, and each cell holds a drawing name ("1", "2", "a"…). The same drawing
  held across several cells is how timing works: on ones, on twos, holds.
- Both views edit the same data, so a change in one shows up in the other at
  once. → `symphony-core::xsheet`.

### 2. The Node view is the real scene
- Each layer is a **Drawing (Element) node**. Data flows top to bottom.
- **Peg** nodes carry transforms. A peg feeds a drawing or another peg, and
  chaining pegs builds the hierarchy of a cut-out rig.
- A **Composite** node stacks its inputs, and the order of its ports sets the
  render order. Separate composites are a Premium feature.
- **Display** feeds the Camera view. **Write** renders to files.
- In the Node view, Drawing nodes run right to left while Timeline layers run
  bottom to top. Mapping one onto the other in the same way as Harmony matters
  for feel. → `symphony-core::graph`.
- Later: Groups (nested graphs with multi-port in/out), effects (blur, glow,
  cutter, tone/highlight, colour override…), and cameras.

### 3. Drawing model (vector first)
- **Pencil** = a vector centreline with variable thickness. Its shape stays
  editable and the thickness can be changed after drawing.
- **Brush** = a filled vector contour. Textured brushes use a greyscale bitmap
  mask applied to the colour.
- Closed shapes can be **auto-filled** with the current paint colour.
- Each drawing has four **art layers**: Overlay, Line Art, Colour Art and
  Underlay. A standard workflow is to draw lines on Line Art and paint on
  Colour Art below it. "Create Colour Art from Line Art" sets this up.
- **Palettes**: strokes and fills reference a *colour ID*, not a raw RGB
  value. Editing a swatch recolours every drawing that uses it. A master
  palette lets a whole team share colours. This is core to the Harmony
  ink & paint workflow, so it belongs in the data model from day one.
- Paint, Ink, Stroke (invisible lines), Close Gap, Cutter, Contour Editor,
  Perspective, Smooth Editor *(verify full tool list)*.
- Bitmap layers also exist, with their own brushes.

### 4. Rigging and deformation
- **Bone**: a skeleton chain where each parent moves its children. It has
  bias and region-of-influence settings.
- **Game Bone**: a simpler bone for game export, with no bias and no
  regions. It doesn't work together with Curve or Envelope.
- **Curve**: a Bézier chain for jointless parts such as hair and faces.
- **Envelope**: a closed loop of points that each move independently. It
  isn't recommended on bitmaps.
- Master controllers, IK, and drawing substitution (swapping the exposed
  drawing for a different hand pose and so on) are central to cut-out work.

### 5. Workspace and views
Camera, Drawing, Timeline, Xsheet, Node, Tool Properties, Colour, Library,
Function (curve editor), Layer Properties, Reference View *(verify)*.
Views are dockable and grouped into saved **workspaces** such as
Default/Drawing/Animation/Rigging *(verify names)*.

### 6. Shortcuts
Harmony ships several shortcut sets: Harmony's own, plus sets modelled on
Flash and on Toon Boom Studio. They can be customised in Preferences.
Shortcuts are function, not expression, so Symphony should offer a
"Harmony-style" keymap. It will be **built from the published default
shortcut list** (docs: "Default Keyboard Shortcuts"), not copied from any
Toon Boom file.

### 7. Scripting
- QtScript/JavaScript API with objects like `column.add(name, type)`, where
  type is one of DRAWING, SOUND, 3DPATH, BEZIER, EASE, EXPR, TIMING,
  QUATERNION, ANNOTATION. There are also `node.*`, `frame.*` and `scene.*`
  objects.
- A Python API (OMC) appeared in recent versions:
  `scene.columns.create("BEZIER", name)`.
- A compatible scripting layer would let studio pipeline scripts carry over.
  This is a long-term goal.

## File formats (interop, later)
| File | What it is | Status |
|---|---|---|
| `.xstage` | XML scene file. It needs its scene folder next to it (elements, palettes). | XML, readable from user files. **No public schema.** |
| `elements/<Name>/*.tvg` | One vector drawing per file. | **Binary, undocumented.** The community decoder (cpsdqs/tvg) only reads metadata so far. |
| `.plt` | Palette. | *(verify format)* |
| Harmony 25 "compact" scenes | Drawings and palettes packed into one file. Not readable by older Harmony. | New. |

Interop strategy: native Symphony format first. Then import from formats
Harmony already **exports** (SVG, PNG sequences, and so on). `.xstage`
import comes later, built only from files users created themselves (see
CLEAN-ROOM.md).

## Existing free tools, and why another one
- **OpenToonz**: powerful, but users report weaker rigging/IK, a less refined
  node compositor, and bugs. Its UI is different enough that Harmony users
  feel lost.
- **Synfig**: vector and bones, but a very different model.
- **Graphite** (Rust): a node-based 2D editor using wgpu. It's good evidence
  that Rust works for this kind of app, and worth studying for
  architecture.
- None of these copies Harmony's *workflow*: the xsheet/timeline pair, the
  peg→drawing→composite graph, line/colour art, and palette-ID paint. That
  workflow is the gap Symphony fills.

## Sources
- What's new in Harmony 27: https://helpcentre.toonboom.com/hc/en-ca/articles/45250935914771-What-s-new-in-Harmony-27
- Harmony 27 release notes: https://docs.toonboom.com/help/harmony-27/premium/release-notes/harmony/harmony-27-release-notes.html
- Harmony 25.2 release notes: https://docs.toonboom.com/help/harmony-25/premium/release-notes/harmony/harmony-25-2-release-notes.html
- About Nodes: https://docs.toonboom.com/help/harmony-20/premium/nodes/about-nodes.html
- Peg nodes: https://docs.toonboom.com/help/harmony-24/premium/nodes/add-peg-nodes.html
- Deformers: https://docs.toonboom.com/help/harmony-25/premium/getting-started/deformation.html
- Game deformation: https://docs.toonboom.com/help/harmony-24/premium/gaming/concept-game-deform-guideline.html
- Drawing: https://docs.toonboom.com/help/harmony-24/premium/getting-started/drawing.html
- Brush tool: https://docs.toonboom.com/help/harmony-24/premium/reference/tool-properties/brush-tool-properties.html
- Art layers: https://docs.toonboom.com/help/harmony-14/premium/paperless-animation/about-art-layer.html
- Shortcuts: https://docs.toonboom.com/help/harmony-24/essentials/keyboard-shortcuts/keyboard-shortcut-section.html
- Shortcut sets: https://docs.toonboom.com/help/harmony-20/paint/keyboard-shortcuts/about-keyboard-shortcut-sets.html
- Scripting (column): https://docs.toonboom.com/help/harmony-24/scripting/script/classcolumn.html
- Python OMC: https://docs.toonboom.com/help/harmony-24/scripting/python/class_o_m_c_1_1_column.html
- Scene structure: https://docs.toonboom.com/help/harmony-25/premium/project-creation/about-file-structure-server.html
- tvg2xml utility: https://docs.toonboom.com/help/harmony-20/premium/utilities/tvg2xml-utility.html
- TVG decoder: https://github.com/cpsdqs/tvg
- Format docs request: https://forums.toonboom.com/t/reference-documentation-of-toon-boom-file-formats/2337
- Graphite: https://github.com/GraphiteEditor/Graphite
- OpenToonz comparison thread: https://groups.google.com/g/opentoonz_en/c/-Sq_DROIfQI
