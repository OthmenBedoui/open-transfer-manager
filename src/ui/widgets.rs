//! Small themed widgets: buttons, switch, segmented control, pills, gradient progress bar.

use super::icons::{self, Icon};
use super::theme::{self, GRADIENT, Palette};
use egui::text::TextWrapping;
use egui::{
    Color32, CornerRadius, FontId, Galley, Mesh, Painter, Rect, Response, Sense, Stroke,
    StrokeKind, Ui, pos2, vec2,
};
use std::sync::Arc;

pub fn elided(
    painter: &Painter,
    text: &str,
    font: FontId,
    color: Color32,
    max_w: f32,
) -> Arc<Galley> {
    let mut job = super::text::job(text, font, color);
    job.wrap = TextWrapping {
        max_width: max_w.max(10.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    painter.layout_job(job)
}

pub fn icon_button(ui: &mut Ui, p: &Palette, icon: Icon, tip: &str, enabled: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(
        vec2(34.0, 34.0),
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    if enabled && resp.hovered() {
        ui.painter().rect_filled(rect, 8, p.hover);
    }
    let color = if !enabled {
        p.border
    } else if resp.hovered() {
        p.text
    } else {
        p.text.gamma_multiply(0.85)
    };
    icons::paint(ui.painter(), rect.shrink(8.0), icon, color);
    let resp = if enabled {
        resp.on_hover_text(tip)
    } else {
        resp
    };
    if enabled {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp
    }
}

#[allow(clippy::too_many_arguments)] // Private drawing primitive; themed wrappers are public.
fn text_button(
    ui: &mut Ui,
    label: &str,
    icon: Option<Icon>,
    fill: Color32,
    hover_fill: Color32,
    text: Color32,
    border: Option<Color32>,
    height: f32,
    font: FontId,
) -> Response {
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, text);
    let icon_w = if icon.is_some() { 22.0 } else { 0.0 };
    let pad = if height >= 36.0 { 16.0 } else { 12.0 };
    let size = vec2(galley.size().x + icon_w + pad * 2.0, height);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let f = if resp.hovered() { hover_fill } else { fill };
    ui.painter().rect_filled(rect, 9, f);
    if let Some(b) = border {
        ui.painter()
            .rect_stroke(rect, 9, Stroke::new(1.0, b), StrokeKind::Inside);
    }
    let mut x = rect.min.x + pad;
    if let Some(i) = icon {
        icons::paint(
            ui.painter(),
            Rect::from_center_size(pos2(x + 8.0, rect.center().y), vec2(16.0, 16.0)),
            i,
            text,
        );
        x += icon_w;
    }
    ui.painter().galley(
        pos2(x, rect.center().y - galley.size().y / 2.0),
        galley,
        text,
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn primary(ui: &mut Ui, p: &Palette, label: &str, icon: Option<Icon>) -> Response {
    text_button(
        ui,
        label,
        icon,
        p.btn,
        p.btn_hover,
        Color32::WHITE,
        None,
        36.0,
        theme::bold(13.5),
    )
}

pub fn secondary(ui: &mut Ui, p: &Palette, label: &str, icon: Option<Icon>) -> Response {
    text_button(
        ui,
        label,
        icon,
        p.surf,
        p.hover,
        p.text,
        Some(p.border),
        32.0,
        theme::font(12.5),
    )
}

pub fn ghost(ui: &mut Ui, p: &Palette, label: &str) -> Response {
    text_button(
        ui,
        label,
        None,
        Color32::TRANSPARENT,
        p.hover,
        p.muted,
        None,
        32.0,
        theme::font(13.0),
    )
}

pub fn switch(ui: &mut Ui, p: &Palette, on: &mut bool) -> Response {
    let (rect, mut resp) = ui.allocate_exact_size(vec2(40.0, 24.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool(resp.id, *on);
    let bg = if *on {
        p.btn
    } else if p.dark {
        p.border
    } else {
        Color32::from_rgb(0xCD, 0xD2, 0xDB)
    };
    ui.painter().rect_filled(rect, 12, bg);
    let x = egui::lerp(rect.left() + 12.0..=rect.right() - 12.0, t);
    ui.painter()
        .circle_filled(pos2(x, rect.center().y), 9.0, Color32::WHITE);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Row with a label (and hint) on the left and a switch on the right.
pub fn switch_row(ui: &mut Ui, p: &Palette, label: &str, hint: &str, on: &mut bool) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(label);
            if !hint.is_empty() {
                ui.label(egui::RichText::new(hint).small().color(p.muted));
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            changed = switch(ui, p, on).changed();
        });
    });
    changed
}

/// Segmented control. Returns true when the value changed.
pub fn segmented<T: PartialEq + Clone>(
    ui: &mut Ui,
    p: &Palette,
    value: &mut T,
    items: &[(T, &str)],
    item_w: f32,
) -> bool {
    let h = 32.0;
    let total = vec2(item_w * items.len() as f32 + 6.0, h + 6.0);
    let (rect, _) = ui.allocate_exact_size(total, Sense::hover());
    ui.painter().rect_filled(rect, 10, p.alt);
    let mut changed = false;
    for (i, (v, label)) in items.iter().enumerate() {
        let r = Rect::from_min_size(
            pos2(rect.min.x + 3.0 + i as f32 * item_w, rect.min.y + 3.0),
            vec2(item_w, h),
        );
        let resp = ui.interact(r, ui.scope_id().with(("seg", i, label)), Sense::click());
        let on = value == v;
        if on {
            ui.painter().rect_filled(r, 8, p.surf);
        } else if resp.hovered() {
            ui.painter().rect_filled(r, 8, p.hover);
        }
        ui.painter().text(
            r.center(),
            egui::Align2::CENTER_CENTER,
            *label,
            if on {
                theme::bold(12.5)
            } else {
                theme::font(12.5)
            },
            if on { p.text } else { p.muted },
        );
        if resp.clicked() && !on {
            *value = v.clone();
            changed = true;
        }
    }
    changed
}

/// Rounded selectable chip ("pill"); returns the click response.
pub fn chip(ui: &mut Ui, p: &Palette, label: &str, selected: bool) -> Response {
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_string(), theme::font(12.5), p.text);
    let size = vec2(galley.size().x + 24.0, 30.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let (bg, fg, border) = if selected {
        (
            theme::tint(p.btn, if p.dark { 0.22 } else { 0.12 }),
            if p.dark {
                Color32::from_rgb(0xC4, 0xB5, 0xFD)
            } else {
                p.btn
            },
            p.btn,
        )
    } else if resp.hovered() {
        (p.hover, p.text, p.border)
    } else {
        (Color32::TRANSPARENT, p.text, p.border)
    };
    ui.painter().rect_filled(rect, 15, bg);
    ui.painter()
        .rect_stroke(rect, 15, Stroke::new(1.0, border), StrokeKind::Inside);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        theme::font(12.5),
        fg,
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn status_pill(
    painter: &Painter,
    left_center: egui::Pos2,
    text: &str,
    fg: Color32,
    bg: Color32,
) {
    let g = painter.layout_no_wrap(text.to_string(), theme::font(12.0), fg);
    let r = Rect::from_min_size(
        pos2(left_center.x, left_center.y - 11.0),
        vec2(g.size().x + 18.0, 22.0),
    );
    painter.rect_filled(r, 11, bg);
    painter.galley(pos2(r.min.x + 9.0, r.center().y - g.size().y / 2.0), g, fg);
}

fn grad_at(t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let (a, b, u) = if t < 0.55 {
        (GRADIENT[0], GRADIENT[1], t / 0.55)
    } else {
        (GRADIENT[1], GRADIENT[2], (t - 0.55) / 0.45)
    };
    a.lerp_to_gamma(b, u)
}

/// Progress bar: track + fill. `gradient` uses the logo colours (active downloads).
pub fn progress_bar(
    painter: &Painter,
    rect: Rect,
    frac: f32,
    track: Color32,
    fill: Option<Color32>,
) {
    let h = rect.height();
    let rad = (h / 2.0) as u8;
    painter.rect_filled(rect, rad, track);
    let frac = frac.clamp(0.0, 1.0);
    if frac <= 0.0 {
        return;
    }
    let w = (rect.width() * frac).max(h);
    let fr = Rect::from_min_size(rect.min, vec2(w, h));
    match fill {
        Some(c) => {
            painter.rect_filled(fr, rad, c);
        }
        None => {
            // gradient across the whole track width, so colours stay put as the bar grows
            let r = h / 2.0;
            painter.circle_filled(pos2(fr.min.x + r, fr.center().y), r, grad_at(0.0));
            painter.circle_filled(
                pos2(fr.max.x - r, fr.center().y),
                r,
                grad_at((fr.max.x - r - rect.min.x) / rect.width()),
            );
            let x0 = fr.min.x + r;
            let x1 = fr.max.x - r;
            if x1 > x0 {
                let mut mesh = Mesh::default();
                let steps = 12;
                for i in 0..=steps {
                    let x = x0 + (x1 - x0) * i as f32 / steps as f32;
                    let c = grad_at((x - rect.min.x) / rect.width());
                    mesh.colored_vertex(pos2(x, fr.min.y), c);
                    mesh.colored_vertex(pos2(x, fr.max.y), c);
                    if i > 0 {
                        let k = (i * 2) as u32;
                        mesh.add_triangle(k - 2, k - 1, k);
                        mesh.add_triangle(k - 1, k + 1, k);
                    }
                }
                painter.add(mesh);
            }
        }
    }
}

/// Small rounded square with the file-type icon and its extension (list rows, format menus).
pub fn type_badge(painter: &Painter, rect: Rect, name_or_ext: &str, dark: bool) {
    let t = crate::model::file_type(name_or_ext);
    let color = theme::type_color(t);
    painter.rect_filled(
        rect,
        CornerRadius::same((rect.width() * 0.25) as u8),
        theme::tint(color, if dark { 0.18 } else { 0.12 }),
    );
    let fg = if dark {
        color.gamma_multiply(1.15)
    } else {
        color
    };
    let icon_size = rect.width() * 0.45;
    let icon_rect = Rect::from_center_size(
        pos2(rect.center().x, rect.min.y + rect.height() * 0.38),
        vec2(icon_size, icon_size),
    );
    icons::paint(painter, icon_rect, icons::for_type(t), fg);
    let ext = crate::model::extension(name_or_ext).to_uppercase();
    let ext: String = if ext.is_empty() {
        "FILE".into()
    } else {
        ext.chars().take(4).collect()
    };
    painter.text(
        pos2(rect.center().x, rect.max.y - rect.height() * 0.2),
        egui::Align2::CENTER_CENTER,
        ext,
        theme::bold(8.5),
        fg,
    );
}

pub fn card(p: &Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(p.surf)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(12)
        .inner_margin(egui::Margin::same(14))
}

pub fn text_input(
    ui: &mut Ui,
    p: &Palette,
    value: &mut String,
    hint: &str,
    width: f32,
) -> Response {
    let r = egui::Frame::new()
        .fill(if p.dark {
            Color32::from_rgb(0x0F, 0x11, 0x17)
        } else {
            Color32::WHITE
        })
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(9)
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.spacing_mut().interact_size.y = 18.0;
            ui.add(
                egui::TextEdit::singleline(value)
                    .hint_text(hint)
                    .frame(egui::Frame::NONE)
                    .desired_width(width)
                    .margin(egui::Margin::ZERO),
            )
        });
    r.inner
}
