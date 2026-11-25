# Contextual keybindings plan

Symphony should make shortcuts easy to work out from the tool's name and the panel where it applies. Start with the actions artists use most often. Every shortcut and gesture should be discoverable from the interface.

## Presets

| Preset | Behavior |
| --- | --- |
| **Symphony** | Default for new installations. Prefer the first letter of ordinary tool and action names. Letters can be reused in different panels when their meanings are clear. |
| **Harmony** | Preserve the existing Harmony mappings in **Preferences → Keybindings**. |
| **Photoshop** | Provide familiar Photoshop bindings for supported drawing and editing actions. Verify the defaults and list the effective bindings in Preferences. |

Keep saved preset choices and user overrides through the update. Show the effective shortcut next to an action in its menu, tool tooltip, and Tab Search result. **Preferences → Keybindings** should show the context, action name, current key, and conflicts. Resolve conflicts within overlapping contexts and keep existing remaps visible.

## Context and text input

- Default to **Under pointer**. An action uses the panel under the mouse, with the last active panel as a fallback when the pointer is over incidental chrome.
- Offer **After interaction** in **Preferences → Keybindings**. Clicking, beginning a gesture, or deliberately moving keyboard focus into a panel activates its shortcuts. Hovering across a panel leaves the current shortcut context in place. Show the active context in the keybinding reference and Search.
- Use the same context resolver for dispatch, shortcut labels, and contextual Search. Opening Search captures the originating context. Dialogs, menus, Search, and other transient overlays get priority for their own input.
- While any text field has focus, printable keys belong to that field, including the Text tool's editor. When the field loses focus, the active panel can use plain-letter shortcuts again.
- A binding can fire only when its action is available in that context. For example, plain **E** chooses Eraser only in Drawing or Camera when the current editing state offers that tool. Pressing it over Node View or Timeline leaves the tool alone.

## First Symphony mapping pass

| Context | Candidate shortcuts | Details |
| --- | --- | --- |
| **Drawing and Camera** | **E** Eraser, **B** Brush, **P** Pencil, **T** Text, **L** Line, **S** Select, **Z** Zoom | Check the available tool group before dispatch. Confirm remaining letters with tooltips and Search before filling gaps. |
| **Node View** | **A** Arrange, **1** Zoom In, **2** Zoom Out, **Space-drag** Pan | Keep empty-space drag for marquee selection and node drag for movement. |
| **Timeline and Xsheet** | **K** Set Key, **D** Duplicate Drawing | Scope drawing clipboard and exposure actions to the cells. Display the linked Clone command beside Duplicate. |
| **Rig posing** | **A** Toggle Anchor, **B** Flip Bend | Only with a rig joint or anchor in the appropriate posing context. **Arrow keys** nudge and **Shift-drag** constrains to a 22.5° direction. |

Use modifiers for consistent variations of the same gesture. **Shift** extends a selection or constrains movement where the tool indicates it. Keep standard OS chords for actions such as Save, Copy, Paste, and Undo. Show contextual modifiers when the tool is active, especially when the pointer is over an eligible handle or target.

## Rollout and acceptance checks

1. Model context and action availability in the shortcut registry. Apply one priority order to global, panel, and rig/tool bindings.
2. Ship the Symphony mapping pass with the hover and interaction context settings. Verify that single letters following a text-edit commit go to the correct panel and that panel crossing behaves as configured.
3. Add Photoshop and audit the Harmony preset against the available actions. Verify menu and Search labels after switching presets or remapping.
4. Test **E** in Drawing, Camera, Node View, Timeline, and focused text. Test the same letter in separate contexts, two bindings in the same context, disabled actions, and keyboard and pointer transitions in a docked workspace.
