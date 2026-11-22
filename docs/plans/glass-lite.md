# Glass-lite: the look Tristan chose (2026-11-22)

Tristan compared neumorphism, claymorphism, glassmorphism and hybrids on
the style-exploration canvas (claude.ai artifact "Symphony style
exploration", artboards 6 and 7). He chose **glass-lite**: today's flat
dark editor plus a light frosted-glass treatment, with a background the
user can choose, at almost no rendering cost.

## How it works

1. **Wallpaper behind the editor, blurred once.** The window background
   is a wallpaper image: a built-in one or the user's own picture. It is
   blurred **once**, when chosen or when the window's size class changes,
   never per frame, and drawn as one stretched texture. Live
   backdrop blur is never used.
2. **Panels are tinted and slightly see-through** over it (about 70% opaque
   panel colour), with a 1 px light border and a faint top highlight. Over a
   blurred wallpaper that reads as frosted glass, for the cost of alpha
   blending.
3. **The drawing is never under glass.** The Camera and Drawing views'
   canvas surround and stage stay opaque and neutral grey, so colours are
   judged truly and the art stays the brightest thing on screen.
4. **Overlays over the canvas are nearly opaque** (about 86%, no blur), so
   they read over any artwork.
5. **Text contrast holds over any wallpaper.** A user's picture is
   darkened and desaturated when imported, so that panel colour over its
   brightest blurred pixel still gives text.primary 4.5:1 and
   text.secondary 4.5:1 (DESIGN D4.4). Check this when the wallpaper is
   chosen; never trust a picture to be dark.
6. **Wallpaper preference**: Black (default for focus), Plain (today),
   Studio (near-neutral), Dusk (soft colour), or Custom picture. On Black
   or Plain the look is a crisper flat look: border, top highlight, soft
   shadow and the active tool's glow.
7. Same everywhere: drawn by us, identical on macOS, Windows and Linux.

## Tokens (from the mockup, to tune in the app)

| Use | Value |
|---|---|
| Panel | `rgba(30,32,36,0.70)`, border `1px rgba(255,255,255,0.09)`, top highlight `inset 0 1px 0 rgba(255,255,255,0.07)`, shadow `0 8px 24px rgba(0,0,0,0.28)`, radius 12 |
| Top bar | `rgba(23,24,28,0.62)` |
| Controls | `rgba(255,255,255,0.08)` with the top highlight, radius 7 |
| Active tool / primary button | `rgba(91,141,230,0.85)`, highlight plus a soft blue glow |
| Inputs, wells | `rgba(0,0,0,0.30)`, `inset 0 1px 2px rgba(0,0,0,0.35)` |
| Canvas overlay (HUD) | `rgba(30,32,36,0.86)`, border `rgba(255,255,255,0.12)` |
| Canvas surround | opaque `#464749` (unchanged) |

## Who

- **Codex** (styling owner): fold this into DESIGN.md (§4 and §6), add the
  tokens and the wallpaper preference, and build it in `symphony-ui` and
  the app chrome. Screenshots of every wallpaper at both review sizes in
  the PR. The wallpaper blur runs once on a background thread (DESIGN
  §17), with a fallback to Plain while it works.
- **Claude**: keep resizing and redrawing cheap (below).

## Performance rules that come with it

- No per-frame blur, ever.
- Resizing a panel must not redraw the artwork: the canvas keeps its scale
  and centre when a view is resized (Harmony's zoom is absolute), and
  stroke meshes are cached in drawing space so frames only move them.
