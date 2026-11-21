//! Design tokens and the egui theme built from them (docs/DESIGN.md §4–6).
//! UI code uses these tokens only. A colour literal anywhere else is a bug (D4.1).

use std::sync::Arc;

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Stroke,
    TextStyle, Vec2,
};

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

// §4.1 colour tokens.
pub const BG_BASE: Color32 = hex(0x1E2025);
pub const BG_PANEL: Color32 = hex(0x272A30);
pub const BG_RAISED: Color32 = hex(0x31353C);
pub const BG_HOVER: Color32 = hex(0x3A3F47);
pub const BG_INPUT: Color32 = hex(0x1A1C20);
pub const LINE_SUBTLE: Color32 = hex(0x3A3E46);
pub const LINE_STRONG: Color32 = hex(0x565B65);
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

// §5.2 type scale.
pub const SIZE_DENSE: f32 = 11.0;
pub const SIZE_BODY: f32 = 13.0;
pub const SIZE_HEADING: f32 = 15.0;

// §6.2 standard sizes.
pub const TOOLBAR_BUTTON: f32 = 24.0;
pub const TOOLS_STRIP: f32 = 36.0;
pub const ROW: f32 = 24.0;
pub const XSHEET_ROW: f32 = 20.0;

// §6.3 radii.
pub const RADIUS_CONTROL: u8 = 3;
pub const RADIUS_FLOATING: u8 = 6;

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
    });
}

/// Dock styling: flat docked panels (D6.3, D6.4), active tab marked in cel blue (D3.2).
pub fn dock_style(style: &egui::Style) -> egui_dock::Style {
    let mut d = egui_dock::Style::from_egui(style);
    d.main_surface_border_stroke = Stroke::NONE;
    d.main_surface_border_rounding = CornerRadius::ZERO;
    d.tab_bar.bg_fill = BG_BASE;
    d.tab_bar.hline_color = LINE_SUBTLE;
    d.tab_bar.corner_radius = CornerRadius::ZERO;
    d.separator.color_idle = BG_BASE;
    d.separator.color_hovered = LINE_STRONG;
    d.separator.color_dragged = ACCENT_SELECT;
    d.separator.width = 4.0;
    d.tab.active.bg_fill = BG_PANEL;
    d.tab.active.text_color = TEXT_PRIMARY;
    d.tab.active.corner_radius = CornerRadius::ZERO;
    d.tab.focused = d.tab.active.clone();
    d.tab.focused.outline_color = ACCENT_SELECT;
    d.tab.inactive.bg_fill = BG_RAISED;
    d.tab.inactive.text_color = TEXT_SECONDARY;
    d.tab.inactive.corner_radius = CornerRadius::ZERO;
    d.tab.hovered.bg_fill = BG_HOVER;
    d.tab.hovered.text_color = TEXT_PRIMARY;
    d.tab.tab_body.bg_fill = BG_PANEL;
    d.tab.tab_body.stroke = Stroke::NONE;
    d.tab.tab_body.corner_radius = CornerRadius::ZERO;
    d.tab.hline_below_active_tab_name = true;
    d
}
