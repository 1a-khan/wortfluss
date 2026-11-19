//! The look of Wortfluss' windows: a dark "glass" panel with rounded corners, cards,
//! pill-shaped choices and icons drawn as vector shapes (no icon font or image files to
//! ship, and no symbol that a font might lack: those showed up as empty boxes).
//!
//! Linux: the window is really translucent (GNOME doesn't blur what is behind an app,
//! so it stays quite opaque to keep the text readable). Windows: opaque, with rounded
//! corners and a dark frame from the system (DWM).

use eframe::egui::{
    self, Color32, CornerRadius, FontId, Pos2, Rect, Response, Sense, Shadow, Stroke, StrokeKind, Ui, Vec2,
};

/// Wortfluss blue (the app icon).
pub const ACCENT: Color32 = Color32::from_rgb(54, 133, 227);
const ACCENT_HOVER: Color32 = Color32::from_rgb(78, 152, 238);
pub const TEXT: Color32 = Color32::from_rgb(236, 239, 245);
pub const TEXT_DIM: Color32 = Color32::from_rgb(150, 158, 175);
pub const DANGER: Color32 = Color32::from_rgb(240, 110, 110);

/// Translucent window on Linux; Windows draws the frame itself (opaque).
pub const TRANSPARENT: bool = cfg!(not(windows));
/// Space around the panel for its shadow (Linux only: there it is drawn by us).
pub const MARGIN: f32 = if TRANSPARENT { 10.0 } else { 0.0 };
const RADIUS: u8 = if TRANSPARENT { 16 } else { 0 };

fn white(alpha: u8) -> Color32 {
    Color32::from_white_alpha(alpha)
}

/// Colours, roundings and spacing for all widgets.
pub fn apply_style(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.override_text_color = Some(TEXT);
    v.panel_fill = Color32::TRANSPARENT;
    v.window_fill = Color32::from_rgb(30, 34, 44); // popups (language list): opaque
    v.window_stroke = Stroke::new(1.0, white(28));
    v.window_corner_radius = CornerRadius::same(12);
    v.menu_corner_radius = CornerRadius::same(12);
    v.popup_shadow = Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(110) };
    v.extreme_bg_color = Color32::from_rgba_unmultiplied(8, 10, 16, 120); // text fields
    v.text_edit_bg_color = Some(Color32::TRANSPARENT);
    v.faint_bg_color = white(8);
    v.selection.bg_fill = ACCENT.gamma_multiply(0.55);
    v.selection.stroke = Stroke::new(1.0, TEXT);
    v.hyperlink_color = ACCENT_HOVER;
    let w = &mut v.widgets;
    for (state, fill, stroke) in [
        (&mut w.noninteractive, Color32::TRANSPARENT, white(18)),
        (&mut w.inactive, white(16), white(22)),
        (&mut w.hovered, white(28), white(40)),
        (&mut w.active, white(36), white(52)),
        (&mut w.open, white(28), white(40)),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = Stroke::new(1.0, stroke);
        state.corner_radius = CornerRadius::same(9);
        state.expansion = 0.0;
    }
    w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_DIM);
    w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    w.hovered.fg_stroke = Stroke::new(1.5, TEXT);
    w.active.fg_stroke = Stroke::new(1.5, TEXT);
    ctx.set_visuals(v);
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = Vec2::new(8.0, 8.0);
        s.spacing.button_padding = Vec2::new(12.0, 6.0);
        s.spacing.interact_size.y = 30.0;
        use egui::TextStyle::*;
        for (style, size) in [(Body, 14.5), (Button, 14.0), (Small, 12.0), (Heading, 17.0), (Monospace, 13.5)] {
            if let Some(f) = s.text_styles.get_mut(&style) {
                f.size = size;
            }
        }
    });
}

/// The window's background: shadow, glass panel with a soft gradient and a fine edge.
/// Returns the panel's rectangle (inside the shadow margin).
pub fn paint_background(ctx: &egui::Context) -> Rect {
    let painter = ctx.layer_painter(egui::LayerId::background());
    let full = ctx.content_rect();
    let panel = full.shrink(MARGIN);
    if TRANSPARENT {
        let shadow = Shadow { offset: [0, 3], blur: (MARGIN * 1.6) as u8, spread: 0, color: Color32::from_black_alpha(120) };
        painter.add(shadow.as_shape(panel, CornerRadius::same(RADIUS)));
    }
    let alpha = if TRANSPARENT { 236 } else { 255 };
    let top = Color32::from_rgba_unmultiplied(36, 42, 57, alpha);
    let bottom = Color32::from_rgba_unmultiplied(17, 20, 28, alpha);
    painter.rect_filled(panel, CornerRadius::same(RADIUS), bottom);
    // Gradient: a mesh over the upper part, fading out.
    let mut mesh = egui::Mesh::default();
    let fade = Rect::from_min_max(panel.min, Pos2::new(panel.max.x, panel.min.y + panel.height() * 0.6));
    let inner = fade.shrink2(Vec2::new(RADIUS as f32 * 0.3, 0.0));
    mesh.colored_vertex(inner.left_top() + Vec2::new(0.0, RADIUS as f32 * 0.3), top);
    mesh.colored_vertex(inner.right_top() + Vec2::new(0.0, RADIUS as f32 * 0.3), top);
    mesh.colored_vertex(inner.right_bottom(), Color32::TRANSPARENT);
    mesh.colored_vertex(inner.left_bottom(), Color32::TRANSPARENT);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(mesh);
    // Glass edge: light on top, fainter all around.
    painter.rect_stroke(panel, CornerRadius::same(RADIUS), Stroke::new(1.0, white(30)), StrokeKind::Inside);
    painter.line_segment(
        [panel.left_top() + Vec2::new(RADIUS as f32, 1.0), panel.right_top() + Vec2::new(-(RADIUS as f32), 1.0)],
        Stroke::new(1.0, white(38)),
    );
    panel
}

/// A frame for content: a slightly lighter pane with a fine edge.
pub fn card() -> egui::Frame {
    egui::Frame::new()
        .fill(white(9))
        .stroke(Stroke::new(1.0, white(18)))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(egui::Margin::symmetric(12, 10))
}

/// Small grey heading with an icon, above a card.
pub fn section_label(ui: &mut Ui, icon: Icon, text: &str) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
        paint_icon(ui.painter(), rect, icon, TEXT_DIM);
        ui.label(egui::RichText::new(text).size(12.5).color(TEXT_DIM));
    });
}

/// Icons, drawn with lines and circles in a 16x16 box.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Icon {
    Globe,
    Back,
    Copy,
    Check,
    Close,
    Replace,
    Sparkle,
    Text,
    Retry,
}

pub fn paint_icon(p: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    let s = rect.width().min(rect.height()) / 16.0;
    let c = rect.center();
    let at = |x: f32, y: f32| Pos2::new(c.x + (x - 8.0) * s, c.y + (y - 8.0) * s);
    let stroke = Stroke::new(1.5 * s.max(0.8), color);
    match icon {
        Icon::Globe => {
            p.circle_stroke(at(8.0, 8.0), 6.5 * s, stroke);
            p.line_segment([at(1.5, 8.0), at(14.5, 8.0)], stroke);
            // meridian: an ellipse, as two arcs
            let arc = |dir: f32| -> Vec<Pos2> {
                (0..=16)
                    .map(|i| {
                        let a = std::f32::consts::PI * (i as f32 / 16.0 - 0.5);
                        at(8.0 + dir * 3.0 * a.cos(), 8.0 + 6.5 * a.sin())
                    })
                    .collect()
            };
            p.line(arc(1.0), stroke);
            p.line(arc(-1.0), stroke);
        }
        Icon::Back => {
            p.line_segment([at(3.0, 8.0), at(13.0, 8.0)], stroke);
            p.line(vec![at(7.5, 3.5), at(3.0, 8.0), at(7.5, 12.5)], stroke);
        }
        Icon::Copy => {
            p.rect_stroke(Rect::from_min_max(at(5.5, 5.5), at(13.5, 13.5)), CornerRadius::same((2.0 * s) as u8), stroke, StrokeKind::Middle);
            p.line(vec![at(10.5, 3.0), at(4.5, 3.0), at(2.5, 3.0), at(2.5, 10.5)], stroke);
        }
        Icon::Check => {
            p.line(vec![at(3.0, 8.5), at(6.5, 12.0), at(13.0, 4.5)], Stroke::new(2.0 * s, color));
        }
        Icon::Close => {
            p.line_segment([at(4.0, 4.0), at(12.0, 12.0)], stroke);
            p.line_segment([at(12.0, 4.0), at(4.0, 12.0)], stroke);
        }
        Icon::Replace => {
            // text lines with an arrow into them
            p.line_segment([at(2.5, 4.0), at(13.5, 4.0)], stroke);
            p.line_segment([at(2.5, 8.0), at(8.0, 8.0)], stroke);
            p.line_segment([at(2.5, 12.0), at(8.0, 12.0)], stroke);
            p.line(vec![at(13.5, 7.0), at(13.5, 12.0), at(10.5, 12.0)], stroke);
            p.line(vec![at(12.0, 10.3), at(10.3, 12.0), at(12.0, 13.7)], stroke);
        }
        Icon::Sparkle => {
            let star = |cx: f32, cy: f32, r: f32| {
                let k = 0.28;
                vec![
                    at(cx, cy - r),
                    at(cx + r * k, cy - r * k),
                    at(cx + r, cy),
                    at(cx + r * k, cy + r * k),
                    at(cx, cy + r),
                    at(cx - r * k, cy + r * k),
                    at(cx - r, cy),
                    at(cx - r * k, cy - r * k),
                ]
            };
            p.add(egui::Shape::convex_polygon(star(6.5, 9.0, 6.0), color, Stroke::NONE));
            p.add(egui::Shape::convex_polygon(star(12.8, 3.6, 2.8), color, Stroke::NONE));
        }
        Icon::Text => {
            p.line_segment([at(2.5, 3.5), at(13.5, 3.5)], stroke);
            p.line_segment([at(2.5, 8.0), at(13.5, 8.0)], stroke);
            p.line_segment([at(2.5, 12.5), at(9.5, 12.5)], stroke);
        }
        Icon::Retry => {
            let pts: Vec<Pos2> = (0..=20)
                .map(|i| {
                    let a = std::f32::consts::TAU * (0.15 + 0.75 * i as f32 / 20.0);
                    at(8.0 + 5.5 * a.cos(), 8.0 + 5.5 * a.sin())
                })
                .collect();
            let end = pts[0];
            p.line(pts, stroke);
            p.line(vec![end + Vec2::new(-3.0 * s, -0.5 * s), end, end + Vec2::new(0.8 * s, -3.0 * s)], stroke);
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    /// Filled with the accent colour: the main action.
    Primary,
    /// Light glass: other actions.
    Secondary,
    /// No background until hovered (header close button).
    Ghost,
}

/// A button with an optional icon in front of the label.
pub fn button(ui: &mut Ui, icon: Option<Icon>, label: &str, kind: Kind, enabled: bool) -> Response {
    let font = FontId::proportional(14.0);
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, TEXT);
    let icon_w = if icon.is_some() { 16.0 + if label.is_empty() { 0.0 } else { 7.0 } } else { 0.0 };
    let pad = if label.is_empty() { Vec2::new(7.0, 7.0) } else { Vec2::new(14.0, 7.0) };
    let size = Vec2::new(galley.size().x + icon_w + 2.0 * pad.x, 32.0_f32.max(galley.size().y + 2.0 * pad.y));
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    if ui.is_rect_visible(rect) {
        let hovered = enabled && response.hovered();
        let pressed = enabled && response.is_pointer_button_down_on();
        let (fill, edge, fg) = match kind {
            Kind::Primary if !enabled => (ACCENT.gamma_multiply(0.35), Color32::TRANSPARENT, white(120)),
            Kind::Primary => (if hovered { ACCENT_HOVER } else { ACCENT }, white(50), Color32::WHITE),
            Kind::Secondary if !enabled => (white(6), white(12), white(90)),
            Kind::Secondary => (white(if pressed { 34 } else if hovered { 26 } else { 14 }), white(if hovered { 44 } else { 24 }), TEXT),
            Kind::Ghost => (white(if hovered { 22 } else { 0 }), Color32::TRANSPARENT, if hovered { TEXT } else { TEXT_DIM }),
        };
        let p = ui.painter();
        p.rect(rect, CornerRadius::same(10), fill, Stroke::new(1.0, edge), StrokeKind::Inside);
        let mut x = rect.left() + pad.x;
        if let Some(i) = icon {
            paint_icon(p, Rect::from_center_size(Pos2::new(x + 8.0, rect.center().y), Vec2::splat(16.0)), i, fg);
            x += icon_w;
        }
        if !label.is_empty() {
            p.galley(Pos2::new(x, rect.center().y - galley.size().y / 2.0), galley, fg);
        }
    }
    if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

/// A pill-shaped choice (tones): filled when selected. `key`: the number to press.
pub fn chip(ui: &mut Ui, key: &str, label: &str, selected: bool) -> Response {
    let font = FontId::proportional(13.5);
    let text = ui.painter().layout_no_wrap(label.to_string(), font.clone(), TEXT);
    let key_g = ui.painter().layout_no_wrap(key.to_string(), FontId::proportional(11.5), TEXT);
    let badge = 18.0;
    let size = Vec2::new(6.0 + badge + 6.0 + text.size().x + 12.0, 32.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();
        let (fill, edge) = if selected {
            (ACCENT, white(60))
        } else {
            (white(if hovered { 24 } else { 11 }), white(if hovered { 40 } else { 20 }))
        };
        let p = ui.painter();
        p.rect(rect, CornerRadius::same(16), fill, Stroke::new(1.0, edge), StrokeKind::Inside);
        let b = Pos2::new(rect.left() + 6.0 + badge / 2.0, rect.center().y);
        p.circle_filled(b, badge / 2.0, if selected { white(60) } else { white(18) });
        p.galley(b - key_g.size() / 2.0, key_g, TEXT);
        p.galley(Pos2::new(b.x + badge / 2.0 + 6.0, rect.center().y - text.size().y / 2.0), text, if selected { Color32::WHITE } else { TEXT });
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The window header: app icon, title, close button; dragging it moves the window.
/// Returns true when the close button was pressed.
pub fn header(ui: &mut Ui, logo: &egui::TextureHandle, title: &str) -> bool {
    // The drag area first, so the close button (added later) gets its clicks.
    let rect = Rect::from_min_size(ui.cursor().min, Vec2::new(ui.available_width(), 32.0));
    let drag = ui.interact(rect, egui::Id::unique("wortfluss-header-drag"), Sense::click_and_drag());
    if drag.drag_started() {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
    let mut close = false;
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
        ui.add(egui::Image::new(logo).fit_to_exact_size(Vec2::splat(22.0)));
        ui.label(egui::RichText::new("Wortfluss").strong().size(15.0));
        ui.label(egui::RichText::new(title).size(14.0).color(TEXT_DIM));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            close = button(ui, Some(Icon::Close), "", Kind::Ghost, true).clicked();
        });
    });
    ui.advance_cursor_after_rect(rect);
    close
}

/// Small grey hint text ("Enter: replace ...") left in a row.
pub fn hint(ui: &mut Ui, text: &str) {
    ui.label(egui::RichText::new(text).size(12.0).color(TEXT_DIM));
}

/// "Thinking" row: three dots that pulse, and a label.
pub fn working(ui: &mut Ui, label: &str) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(30.0, 16.0), Sense::hover());
        let t = ui.input(|i| i.time) as f32;
        for k in 0..3 {
            let phase = (t * 4.0 - k as f32 * 0.7).sin() * 0.5 + 0.5;
            let pos = Pos2::new(rect.left() + 5.0 + k as f32 * 10.0, rect.center().y);
            ui.painter().circle_filled(pos, 2.5 + phase * 1.2, ACCENT.gamma_multiply(0.45 + 0.55 * phase));
        }
        ui.label(egui::RichText::new(label).color(TEXT_DIM));
        ui.ctx().request_repaint();
    });
}
