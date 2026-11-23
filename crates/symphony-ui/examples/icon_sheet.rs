//! Write Symphony's icon set as one SVG sheet, to edit the icons or draw
//! new ones in a vector editor:
//!
//!     cargo run -p symphony-ui --example icon_sheet -- icons.svg
//!
//! Each icon is painted as the app paints it (`icons::paint`) on its
//! 16 x 16 grid, read back as shapes, and written as one group named
//! "Icons / <Category> / <name>", with a 16 x 16 board and the Rust name
//! in `data-icon`. Line strokes are 1.5 units with round caps and joins
//! (docs/DESIGN.md D7.2). Labels sit in their own group, outside the icons.

use egui::{Color32, Pos2, Rect, Shape};
use symphony_ui::icons::{self, Icon};

const CATEGORIES: [(&str, &[Icon]); 5] = [
    (
        "Tools",
        &[
            Icon::Select,
            Icon::Transform,
            Icon::Brush,
            Icon::Pencil,
            Icon::Eraser,
            Icon::Paint,
            Icon::Dropper,
            Icon::Hand,
            Icon::Zoom,
            Icon::ZoomOut,
            Icon::CloseGap,
            Icon::LineEdit,
        ],
    ),
    (
        "Playback",
        &[
            Icon::FirstFrame,
            Icon::PrevFrame,
            Icon::Play,
            Icon::Pause,
            Icon::NextFrame,
            Icon::LastFrame,
        ],
    ),
    (
        "Layers",
        &[
            Icon::Eye,
            Icon::EyeOff,
            Icon::Lock,
            Icon::Unlock,
            Icon::Onion,
            Icon::ArtPreview,
            Icon::ArtOverlay,
            Icon::ArtLine,
            Icon::ArtColour,
            Icon::ArtUnderlay,
        ],
    ),
    ("View", &[Icon::Fit, Icon::RotateLeft, Icon::RotateRight]),
    (
        "Actions",
        &[
            Icon::Plus,
            Icon::Minus,
            Icon::AddKeyframe,
            Icon::RemoveKeyframe,
            Icon::Trash,
            Icon::Forbidden,
        ],
    ),
];

/// Icon cell pitch and the room for a category title and the labels.
const PITCH_X: f32 = 28.0;
const PITCH_Y: f32 = 40.0;
const TITLE: f32 = 8.0;
const MARGIN: f32 = 8.0;
/// The round join dot `icons::line` draws at every vertex.
const JOIN: f32 = 0.75;
const EPS: f32 = 1e-3;

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "icons.svg".into());
    let listed: usize = CATEGORIES.iter().map(|(_, l)| l.len()).sum();
    assert_eq!(listed, Icon::ALL.len(), "every icon has a category");
    for icon in Icon::ALL {
        assert!(
            CATEGORIES.iter().any(|(_, l)| l.contains(&icon)),
            "{icon:?} has no category"
        );
    }

    let ctx = egui::Context::default();
    let cols = CATEGORIES.iter().map(|(_, l)| l.len()).max().unwrap_or(1);
    let width = MARGIN * 2.0 + cols as f32 * PITCH_X;
    let height = MARGIN * 2.0 + CATEGORIES.len() as f32 * PITCH_Y;
    let mut icons_svg = String::new();
    let mut labels = String::new();
    for (row, (category, list)) in CATEGORIES.iter().enumerate() {
        let y = MARGIN + row as f32 * PITCH_Y;
        labels += &format!(
            "    <text x=\"{MARGIN}\" y=\"{:.2}\" font-size=\"4\" font-weight=\"bold\">{category}</text>\n",
            y + 4.0
        );
        for (col, &icon) in list.iter().enumerate() {
            let x = MARGIN + col as f32 * PITCH_X;
            let top = y + TITLE;
            let name = snake(&format!("{icon:?}"));
            icons_svg += &format!(
                "  <g id=\"Icons / {category} / {name}\" data-icon=\"{icon:?}\" transform=\"translate({x} {top})\">\n    <rect class=\"board\" width=\"16\" height=\"16\" fill=\"none\"/>\n"
            );
            for el in svg_shapes(&ctx, icon) {
                icons_svg += &format!("    {el}\n");
            }
            icons_svg += "  </g>\n";
            labels += &format!(
                "    <text x=\"{:.2}\" y=\"{:.2}\" font-size=\"3\" text-anchor=\"middle\">{name}</text>\n",
                x + 8.0,
                top + 21.0
            );
        }
    }
    let svg = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!-- Symphony icon set, written by crates/symphony-ui/examples/icon_sheet.rs\n\
     from crates/symphony-ui/src/icons.rs. Each icon: 16 x 16 units, keep\n\
     shapes inside 2..14, 1.5-unit strokes with round caps and joins,\n\
     outline style, fill only where it means something (DESIGN.md D7).\n\
     Use straight lines, circular arcs and circles: the code draws those. -->\n\
<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {width} {height}\">\n\
  <rect id=\"page\" width=\"{width}\" height=\"{height}\" fill=\"#FFFFFF\"/>\n\
{icons_svg}\
  <g id=\"Labels\" font-family=\"sans-serif\" fill=\"#666666\">\n{labels}  </g>\n\
</svg>\n",
        w = width * 8.0,
        h = height * 8.0,
    );
    std::fs::write(&out, svg).expect("write the sheet");
    println!("{} icons -> {out}", Icon::ALL.len());
}

fn snake(name: &str) -> String {
    let mut s = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            s.push('_');
        }
        s.extend(c.to_lowercase());
    }
    s
}

/// The icon as the app paints it on a 16-unit square, as SVG elements.
fn svg_shapes(ctx: &egui::Context, icon: Icon) -> Vec<String> {
    let mut shapes = Vec::new();
    let mut frame = ctx.run_ui(egui::RawInput::default(), |ui| {
        let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("icon-sheet"));
        let p = ui.ctx().layer_painter(layer);
        icons::paint(
            &p,
            Rect::from_min_size(Pos2::ZERO, egui::vec2(16.0, 16.0)),
            icon,
            Color32::BLACK,
        );
        ui.ctx().graphics(|g| {
            if let Some(list) = g.get(layer) {
                for c in list.all_entries() {
                    flatten(&c.shape, &mut shapes);
                }
            }
        });
    });
    frame.textures_delta.clear();
    to_svg(&shapes)
}

fn flatten(shape: &Shape, out: &mut Vec<Shape>) {
    match shape {
        Shape::Vec(v) => v.iter().for_each(|s| flatten(s, out)),
        Shape::Noop => {}
        s => out.push(s.clone()),
    }
}

fn same(a: Pos2, b: Pos2) -> bool {
    (a - b).length() < EPS
}

fn num(v: f32) -> String {
    let r = (v * 1000.0).round() / 1000.0;
    let s = format!("{r:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// Line segments that meet end to start become one polyline (a closing
/// segment back to the start makes it a polygon); the round-join dots at
/// its vertices are dropped, since SVG joins are round already.
fn to_svg(shapes: &[Shape]) -> Vec<String> {
    let stroke = "fill=\"none\" stroke=\"#000000\" stroke-width=\"1.5\" stroke-linecap=\"round\" stroke-linejoin=\"round\"";
    let mut out = Vec::new();
    let mut chain: Vec<Pos2> = Vec::new();
    let mut closed = false;
    let mut done: Vec<Pos2> = Vec::new();
    let flush =
        |chain: &mut Vec<Pos2>, closed: &mut bool, out: &mut Vec<String>, done: &mut Vec<Pos2>| {
            if chain.len() >= 2 {
                let pts: Vec<String> = chain
                    .iter()
                    .map(|p| format!("{},{}", num(p.x), num(p.y)))
                    .collect();
                let tag = if *closed { "polygon" } else { "polyline" };
                out.push(format!("<{tag} points=\"{}\" {stroke}/>", pts.join(" ")));
            }
            done.clone_from(chain);
            chain.clear();
            *closed = false;
        };
    for s in shapes {
        match s {
            Shape::LineSegment { points: [a, b], .. } => {
                let (a, b) = (*a, *b);
                if chain.last().is_some_and(|l| same(*l, a)) && !closed {
                    if chain.len() > 2 && same(chain[0], b) {
                        closed = true;
                    } else {
                        chain.push(b);
                    }
                } else if chain.len() > 2
                    && !closed
                    && same(chain[0], a)
                    && chain.last().is_some_and(|l| same(*l, b))
                {
                    // `icons::closed` ends with a segment from the first
                    // vertex to the last.
                    closed = true;
                } else {
                    flush(&mut chain, &mut closed, &mut out, &mut done);
                    chain = vec![a, b];
                }
            }
            Shape::Circle(c)
                if (c.radius - JOIN).abs() < EPS
                    && c.fill != Color32::TRANSPARENT
                    && chain.iter().chain(done.iter()).any(|v| same(*v, c.center)) =>
            {
                // A round-join dot on a vertex: SVG's round joins draw it.
            }
            Shape::Circle(c) => {
                flush(&mut chain, &mut closed, &mut out, &mut done);
                if c.fill != Color32::TRANSPARENT {
                    out.push(format!(
                        "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"#000000\"/>",
                        num(c.center.x),
                        num(c.center.y),
                        num(c.radius)
                    ));
                } else {
                    out.push(format!(
                        "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" {stroke}/>",
                        num(c.center.x),
                        num(c.center.y),
                        num(c.radius)
                    ));
                }
            }
            Shape::Path(p) => {
                flush(&mut chain, &mut closed, &mut out, &mut done);
                let pts: Vec<String> = p
                    .points
                    .iter()
                    .map(|q| format!("{},{}", num(q.x), num(q.y)))
                    .collect();
                out.push(format!(
                    "<polygon points=\"{}\" fill=\"#000000\"/>",
                    pts.join(" ")
                ));
            }
            other => {
                flush(&mut chain, &mut closed, &mut out, &mut done);
                out.push(format!("<!-- shape not written: {other:?} -->"));
            }
        }
    }
    flush(&mut chain, &mut closed, &mut out, &mut done);
    out
}
