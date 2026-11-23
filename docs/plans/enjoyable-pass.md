# Making Symphony enjoyable: the small things (pass, 2026-11-23)

Tristan: "there are still many little things missing for it to be
enjoyable to use." This pass went through the app the way an animator
would: every view on screen, every command and shortcut, and a check in
the code for the everyday affordances other animation apps have.
"Seen" means spotted on screen; "checked" means confirmed in the code.

## A. Feel and feedback (the missing "sparkle")

Opening and closing are plain fades now (Tristan's call), so what's left
to make the app feel alive is feedback that answers each action. Each
item follows DESIGN §9: short, never blocking input.

1. **Undo and redo show where.** After undo or redo, briefly outline what
   changed on the canvas, and name it in the status bar (the name is
   already there). Today an undo can happen off-screen with no sign.
2. **Saved, visibly.** A short "Saved" (or "Autosaved") in the status bar
   that fades. The title marks unsaved changes, but nothing confirms a
   save (checked: autosave runs every 120 s, silently).
3. **Tool switch feedback.** Changing tool by shortcut gives a brief tool
   name near the cursor (as Harmony does in its tool tip), since the
   toolbar is far from where you look.
4. **Brush size change at the cursor.** While resizing the brush with keys
   or a drag, show the size ring and number at the cursor, not only in
   Tool Properties.
5. **The playhead glides.** Clicking a frame moves the playhead with the
   Select motion instead of jumping; while playing, it stays exact.
6. **Layers animate in, out and when reordered** (Expand and Settle), so
   adding or deleting a layer doesn't make rows jump.
7. **Pressed and hover states.** Buttons have hover and selected looks,
   but no pressed state; a quick darken on press makes clicks feel solid.

## B. Drawing workflow (daily use)

8. **Flip the view** (mirror, as on a light box): no command exists
   (checked). Artists flip constantly to check drawings. It's view only,
   not the art.
9. **Light table and onion skin look:** onion skin has no colour or
   opacity settings (checked). Add previous/next tints, an opacity
   slider, and Harmony's light table (other layers washed out).
10. **Reorder layers by dragging** in the Timeline and Xsheet (checked:
    nothing does it today).
11. **Hold-for-tool:** Space for Hand exists (checked); add hold Alt for
    the Dropper (Harmony) and hold Shift+Space to rotate the view.
12. **Stroke stabiliser per tool,** shown with a lag-line preview: the
    smoothing value exists, but there's no visual cue.
13. **Field guide or grid overlay** (checked: none), including the safe
    area.

## C. Timeline, Xsheet and playback

14. **Playback range:** set in and out points, and loop just that range
    (checked: playback always loops the whole scene).
15. **Xsheet uses its space:** it shows three narrow columns crammed left
    (seen). Columns should fill the width, with drawing names in cells
    and a hold line for exposures, Harmony-style.
16. **Same transport everywhere:** the Timeline and Xsheet bars differ
    (seen: the Xsheet only has Play). Use one transport: first, previous,
    play, next, last and loop.
17. **Peg tracks cut off:** "Head-P" rows are cut at the bottom with no
    sign there's more (seen). They need a scroll fade like the search
    list.
18. **Sound:** no audio track or scrubbing (checked). Lip sync and timing
    to music need it; it's a bigger task, noted for the roadmap.

## D. Readability and polish

19. **Preferences is too see-through:** the drawing shows through its
    text on the black wallpaper (seen). Glass windows over the canvas
    need a denser tint when there's no blurred wallpaper behind them.
20. **The Line swatch vanishes:** a near-black swatch on the dark panel is
    invisible (seen). Swatches need a thin rim.
21. **Cut-off tool text:** the Eraser's options end mid-sentence with no
    hint that the panel scrolls (seen). Fade the bottom edge.
22. **The Camera toolbar spends its best space on onion counts**
    ("Previous − 1 +", "Next − 1 +", seen). Collapse them into the onion
    button's menu and keep the bar for view and drawing controls.
23. **"AI" in Preferences reads like an AI feature:** it's the AI-free
    provenance page. Rename it "AI-free" or "Provenance".
24. **Empty states:** an empty drawing, layer or Node view should say what
    to do next in one line (Drawing view, frame without a drawing).

## Suggested order

First batch (most felt per hour of work): 19, 20, 21, 1, 2, 3, 4, 8, 9,
14. Then 15, 16, 17, 10, 22, 5, 6, 7. Bigger ones later: 18 (sound), 12,
13.
