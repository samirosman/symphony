//! Design tokens and the egui theme built from them (docs/DESIGN.md §4–6).
//! UI code uses these tokens only. A colour literal anywhere else is a bug (D4.1).

use std::sync::Arc;

use egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Stroke,
    TextStyle, Vec2,
};

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

// §4.1 colour tokens.
pub const BG_BASE: Color32 = hex(0x16181C);
pub const BG_PANEL: Color32 = hex(0x272A30);
pub const BG_RAISED: Color32 = hex(0x31353C);
pub const BG_HOVER: Color32 = hex(0x3A3F47);
pub const BG_INPUT: Color32 = hex(0x1A1C20);
pub const LINE_SUBTLE: Color32 = hex(0x3A3E46);
pub const LINE_STRONG: Color32 = hex(0x565B65);
/// Edges of controls that would otherwise vanish (checkbox, radio): 3:1 or more on every surface (D4.4).
pub const LINE_CONTROL: Color32 = hex(0x80868F);
pub const TEXT_PRIMARY: Color32 = hex(0xDDE0E6);
pub const TEXT_SECONDARY: Color32 = hex(0xA0A5AF);
pub const TEXT_DISABLED: Color32 = hex(0x686D77);
pub const ON_ACCENT: Color32 = hex(0xFFFFFF);
pub const ACCENT_SELECT: Color32 = hex(0x6A95F0);
pub const ACCENT_SELECT_FILL: Color32 = hex(0x3D63B8);
pub const ACCENT_TIME: Color32 = hex(0xF0A63A);
pub const STATE_DANGER: Color32 = hex(0xE86A62);
pub const STATE_OK: Color32 = hex(0x62C484);
pub const CANVAS_SURROUND: Color32 = hex(0x505359);
pub const CANVAS_STAGE: Color32 = hex(0xFFFFFF);
/// Onion skin tints for previous and next drawings (DESIGN §4.3). Defaults
/// follow the red-before, green-after convention; to verify against Harmony 27.
pub const ONION_BEFORE: Color32 = hex(0xE0483E);
pub const ONION_AFTER: Color32 = hex(0x2FA35A);
/// Neutral shadow under the stage (D3.10) and floating windows (D6.4).
pub const SHADOW: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 150);

// §5.2 type scale.
pub const SIZE_DENSE: f32 = 11.0;
pub const SIZE_BODY: f32 = 13.0;
pub const SIZE_HEADING: f32 = 15.0;

// §6.2 standard sizes.
pub const TOP_BAR: f32 = 40.0;
pub const TAB_ROW: f32 = 32.0;
pub const STATUS_BAR: f32 = 24.0;
pub const TOOLBAR_BUTTON: f32 = 24.0;
pub const TOOL_BUTTON: f32 = 32.0;
pub const TOOL_ICON: f32 = 20.0;
pub const TOOL_GROUP_GAP: f32 = 12.0;
pub const TOOLS_STRIP: f32 = 44.0;
pub const ROW: f32 = 28.0;
pub const XSHEET_ROW: f32 = 20.0;
pub const FRAME_CELL: f32 = 16.0;
pub const CARD_PAD: i8 = 12;
pub const GUTTER: f32 = 6.0;

// §6.3 radii.
pub const RADIUS_CONTROL: u8 = 4;
pub const RADIUS_CARD: u8 = 8;
pub const RADIUS_FLOATING: u8 = 8;

pub fn dense() -> FontId {
    FontId::proportional(SIZE_DENSE)
}

pub fn body() -> FontId {
    FontId::proportional(SIZE_BODY)
}

/// Digits in time grids line up (D5.1).
pub fn numbers() -> FontId {
    FontId::monospace(SIZE_DENSE)
}

fn fonts() -> FontDefinitions {
    let mut f = FontDefinitions::default();
    f.font_data.insert(
        "Inter".into(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/fonts/Inter.ttf"
        ))),
    );
    f.font_data.insert(
        "JetBrainsMono".into(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/fonts/JetBrainsMono.ttf"
        ))),
    );
    f.families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "Inter".into());
    f.families
        .entry(FontFamily::Monospace)
        .or_default()
        .insert(0, "JetBrainsMono".into());
    f
}

fn widget(bg: Color32, fg: Color32, stroke: Stroke) -> egui::style::WidgetVisuals {
    egui::style::WidgetVisuals {
        bg_fill: bg,
        weak_bg_fill: bg,
        bg_stroke: stroke,
        corner_radius: CornerRadius::same(RADIUS_CONTROL),
        fg_stroke: Stroke::new(1.0, fg),
        expansion: 0.0,
    }
}

pub fn apply(ctx: &egui::Context) {
    ctx.set_fonts(fonts());
    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (TextStyle::Small, FontId::proportional(SIZE_DENSE)),
            (TextStyle::Body, FontId::proportional(SIZE_BODY)),
            (TextStyle::Button, FontId::proportional(SIZE_BODY)),
            (TextStyle::Heading, FontId::proportional(SIZE_HEADING)),
            (TextStyle::Monospace, FontId::monospace(SIZE_DENSE)),
        ]
        .into();

        let s = &mut style.spacing;
        s.item_spacing = Vec2::new(8.0, 4.0);
        s.button_padding = Vec2::new(8.0, 4.0);
        s.interact_size = Vec2::new(24.0, 20.0);
        s.window_margin = Margin::same(8);
        s.menu_margin = Margin::same(4);

        let v = &mut style.visuals;
        v.dark_mode = true;
        v.override_text_color = None;
        v.panel_fill = BG_PANEL;
        v.window_fill = BG_RAISED;
        v.window_stroke = Stroke::new(1.0, LINE_STRONG);
        v.window_corner_radius = CornerRadius::same(RADIUS_FLOATING);
        v.menu_corner_radius = CornerRadius::same(RADIUS_FLOATING);
        v.extreme_bg_color = BG_INPUT;
        v.faint_bg_color = BG_RAISED;
        v.code_bg_color = BG_INPUT;
        v.hyperlink_color = ACCENT_SELECT;
        v.warn_fg_color = ACCENT_TIME;
        v.error_fg_color = STATE_DANGER;
        v.selection.bg_fill = ACCENT_SELECT_FILL;
        v.selection.stroke = Stroke::new(1.0, ON_ACCENT);
        v.widgets.noninteractive = widget(BG_PANEL, TEXT_PRIMARY, Stroke::new(1.0, LINE_SUBTLE));
        v.widgets.inactive = widget(BG_RAISED, TEXT_PRIMARY, Stroke::NONE);
        v.widgets.hovered = widget(BG_HOVER, TEXT_PRIMARY, Stroke::new(1.0, LINE_STRONG));
        v.widgets.active = widget(
            ACCENT_SELECT_FILL,
            ON_ACCENT,
            Stroke::new(1.0, ACCENT_SELECT),
        );
        v.widgets.open = widget(BG_RAISED, TEXT_PRIMARY, Stroke::new(1.0, LINE_STRONG));
        // Sliders: the filled part shows the value at a glance, and the handle is a round knob.
        v.slider_trailing_fill = true;
        v.handle_shape = egui::style::HandleShape::Circle;
    });
}

#[cfg(feature = "dock")]
/// Dock styling (DESIGN §3.1): every view is a card on `bg.base` with
/// 6 px gutters. Tabs are flat, with the active one on a raised tone and the
/// focused view's tab outlined in cel blue (D3.2). No close or collapse
/// buttons (D3.4).
pub fn dock_style(style: &egui::Style) -> egui_dock::Style {
    use egui_dock::TabInteractionStyle;
    let top = CornerRadius {
        nw: RADIUS_CARD,
        ne: RADIUS_CARD,
        sw: 0,
        se: 0,
    };
    let bottom = CornerRadius {
        nw: 0,
        ne: 0,
        sw: RADIUS_CARD,
        se: RADIUS_CARD,
    };
    let tab_radius = CornerRadius::same(RADIUS_CONTROL);
    let tab = |bg: Color32, text: Color32, outline: Color32| TabInteractionStyle {
        outline_color: outline,
        corner_radius: tab_radius,
        bg_fill: bg,
        text_color: text,
    };

    let mut d = egui_dock::Style::from_egui(style);
    d.dock_area_padding = Some(Margin::same(GUTTER as i8));
    d.main_surface_border_stroke = Stroke::NONE;
    d.main_surface_border_rounding = CornerRadius::ZERO;
    d.separator.width = GUTTER;
    d.separator.extra = 0.0;
    d.separator.color_idle = BG_BASE;
    d.separator.color_hovered = LINE_STRONG;
    d.separator.color_dragged = ACCENT_SELECT;
    d.tab_bar.bg_fill = BG_PANEL;
    d.tab_bar.height = TAB_ROW;
    d.tab_bar.corner_radius = top;
    d.tab_bar.hline_color = LINE_SUBTLE;
    d.tab_bar.show_scroll_bar_on_overflow = false;
    d.tab_bar.inner_margin = Margin {
        left: 6,
        right: 6,
        top: 4,
        bottom: 4,
    };
    d.tab.active = tab(BG_RAISED, TEXT_PRIMARY, Color32::TRANSPARENT);
    d.tab.inactive = tab(BG_PANEL, TEXT_SECONDARY, Color32::TRANSPARENT);
    d.tab.hovered = tab(BG_RAISED, TEXT_PRIMARY, Color32::TRANSPARENT);
    d.tab.focused = tab(BG_RAISED, TEXT_PRIMARY, ACCENT_SELECT);
    d.tab.active_with_kb_focus = tab(BG_RAISED, TEXT_PRIMARY, ACCENT_SELECT);
    d.tab.inactive_with_kb_focus = tab(BG_PANEL, TEXT_PRIMARY, ACCENT_SELECT);
    d.tab.focused_with_kb_focus = tab(BG_RAISED, TEXT_PRIMARY, ACCENT_SELECT);
    d.tab.hline_below_active_tab_name = false;
    d.tab.tab_body.bg_fill = BG_PANEL;
    d.tab.tab_body.stroke = Stroke::NONE;
    d.tab.tab_body.corner_radius = bottom;
    d.tab.tab_body.inner_margin = Margin::same(0);
    d.overlay.selection_color = ACCENT_SELECT.gamma_multiply(0.25);
    d
}
