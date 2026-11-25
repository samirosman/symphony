# Symphony icon inventory

Use the code names as labels for replacement artwork. The editor defines 38 named glyphs. Four frame-navigation glyphs currently appear only on the developer icon sheet. The current masters use a 16 × 16 grid, single-colour artwork, and rounded 1.5-unit strokes. The theme supplies normal, active, and disabled colours. See [icon definitions](../../crates/symphony-ui/src/icons.rs) and **Help → Icon sheet** in a debug build.

## Drawing tools

| Code name | Current visual | Job |
| --- | --- | --- |
| `Select` | Pointer arrow | Selects strokes and fills. |
| `Transform` | Box with corner handles | Moves, rotates, and scales a layer. Also a cursor badge. |
| `Brush` | Paintbrush | Brush tool in bitmap-style mode and brush cursor badge. |
| `Pen` | Pen nib | Brush tool in Vector mode. |
| `Line` | Segment with endpoints | Draws a straight or curved line. |
| `Text` | Capital T | Places and edits text. |
| `Pencil` | Pencil | Draws editable pencil lines. |
| `Eraser` | Eraser | Erases ink or painted regions. Also a cursor badge. |
| `Paint` | Paint bucket | Fills closed artwork or recolours paint and lines. |
| `Dropper` | Eyedropper | Samples a colour. |
| `CloseGap` | Line ends with a dotted bridge | Adds an invisible boundary between line ends. |
| `LineEdit` | Curve with a point being pulled | Pushes or moves stroke points. |
| `Hand` | Hand | Pans the canvas. Also appears during temporary pan. |
| `Zoom` | Magnifier with plus | Zoom tool and zoom-in cursor. |
| `ZoomOut` | Magnifier with minus | Zoom-out cursor. |
| `Forbidden` | Slashed circle | Indicates that a locked layer blocks a drawing tool. |

## Timeline and shared controls

| Code name | Current visual | Job |
| --- | --- | --- |
| `Play` | Right triangle | Starts playback. |
| `Pause` | Two vertical bars | Pauses playback. |
| `Eye` | Open eye | Shows that a layer is visible. Click to hide. |
| `EyeOff` | Closed eye | Shows that a layer is hidden. Click to show. |
| `AddKeyframe` | Diamond with plus | Sets a keyframe at the playhead. |
| `RemoveKeyframe` | Diamond with minus | Removes a keyframe at the playhead. |
| `Trash` | Trash can | Deletes a Timeline layer. |
| `Plus` | Plus sign | Adds a layer, colour, or brush preset, or increases onion-skin count. |
| `Minus` | Minus sign | Removes a colour or brush preset, or decreases onion-skin count. |
| `FirstFrame` | Left triangle and bar | Go to first frame. Developer sheet only. |
| `PrevFrame` | Left chevron | Previous frame. Developer sheet only. |
| `NextFrame` | Right chevron | Next frame. Developer sheet only. |
| `LastFrame` | Right triangle and bar | Go to last frame. Developer sheet only. |

## Canvas and art layers

| Code name | Current visual | Job |
| --- | --- | --- |
| `Onion` | Three overlapping squares | Shows neighbouring drawings as onion skin. |
| `Fit` | Framing corners around a square | Resets the canvas zoom, rotation, and pan. |
| `RotateLeft` | Counterclockwise circular arrow | Rotates the canvas counterclockwise. |
| `RotateRight` | Clockwise circular arrow | Rotates the canvas clockwise. Also a cursor badge for rotation handles. |
| `ArtPreview` | Outlined stack of sheets | Previews the other art layers. |
| `ArtOverlay` | Stack with top sheet filled | Switches to Overlay. |
| `ArtLine` | Outlined circle | Switches to Line Art. |
| `ArtColour` | Filled circle | Switches to Colour Art. |
| `ArtUnderlay` | Stack with bottom sheet filled | Switches to Underlay. |

## Artwork outside the icon set

- The **Symphony S mark** appears in the editor, launcher, and welcome screen. macOS uses the ICNS for the app bundle. [PNG](../../crates/symphony-app/assets/Symphony.png) · [ICNS](../../crates/symphony-app/assets/Symphony.icns)
- The **Timeline keyframe marker** is a small amber diamond on the track. [Timeline drawing code](../../crates/symphony-app/src/views/timing.rs)
- **Graph and rig marks** include image and transform sockets, bone endpoints, the square IK target, and Mesh Warp handles. [Node View](../../crates/symphony-app/src/views/node.rs) · [Rig handles](../../crates/symphony-app/src/views/rig_edit.rs) · [Mesh Warp handles](../../crates/symphony-app/src/views/warp_edit.rs)
- **Interface chrome** includes tool and docking grips, disclosure chevrons, dropdown arrows, checkboxes, and standard grab, resize, and text cursors.
