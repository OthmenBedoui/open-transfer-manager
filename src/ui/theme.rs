//! Colours, fonts and egui style for the light and dark themes (from the approved design).

use crate::model::FileType;
use egui::{
    Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle,
};
use std::sync::Arc;

#[derive(Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    pub bg: Color32,
    pub side: Color32,
    pub surf: Color32,
    pub alt: Color32,
    pub hover: Color32,
    pub border: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub btn: Color32,
    pub btn_hover: Color32,
    pub soft: Color32,
    pub sel: Color32,
    pub success: Color32,
    pub danger: Color32,
    pub warn: Color32,
    pub track: Color32,
}

const fn hex(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

pub const GRADIENT: [Color32; 3] = [hex(0xFF0A8C), hex(0x7C3AED), hex(0x00C8F0)];

pub fn gold(dark: bool) -> Color32 {
    if dark { hex(0xE4BE55) } else { hex(0x896000) }
}

pub fn palette(dark: bool) -> Palette {
    if dark {
        Palette {
            dark,
            bg: hex(0x0F1117),
            side: hex(0x0B0D12),
            surf: hex(0x161922),
            alt: hex(0x1E2230),
            hover: hex(0x232838),
            border: hex(0x262B38),
            text: hex(0xE8EAF0),
            muted: hex(0x8A93A6),
            accent: hex(0xA78BFA),
            btn: hex(0x7C3AED),
            btn_hover: hex(0x8B5CF6),
            soft: hex(0x241B47),
            sel: hex(0x1F1A3A),
            success: hex(0x22C55E),
            danger: hex(0xF87171),
            warn: hex(0xFBBF24),
            track: hex(0x262B38),
        }
    } else {
        Palette {
            dark,
            bg: hex(0xF6F7F9),
            side: hex(0xEEF0F4),
            surf: hex(0xFFFFFF),
            alt: hex(0xF1F3F7),
            hover: hex(0xE7EAF0),
            border: hex(0xE2E5EB),
            text: hex(0x14161C),
            muted: hex(0x5F6778),
            accent: hex(0x6D28D9),
            btn: hex(0x6D28D9),
            btn_hover: hex(0x5B21B6),
            soft: hex(0xEDE9FE),
            sel: hex(0xF4F0FF),
            success: hex(0x15803D),
            danger: hex(0xDC2626),
            warn: hex(0xB45309),
            track: hex(0xE4E7EC),
        }
    }
}

pub fn type_color(t: FileType) -> Color32 {
    match t {
        FileType::Video => hex(0xEF4444),
        FileType::Audio => hex(0xA855F7),
        FileType::Disk => hex(0x06B6D4),
        FileType::Archive => hex(0xF59E0B),
        FileType::Document => hex(0x3B82F6),
        FileType::Image => hex(0x14B8A6),
        FileType::Program | FileType::Other => hex(0x64748B),
    }
}

pub fn tint(c: Color32, alpha: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (alpha * 255.0) as u8)
}

pub fn semibold() -> FontFamily {
    FontFamily::Name("semibold".into())
}

pub fn font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

pub fn bold(size: f32) -> FontId {
    FontId::new(size, semibold())
}

pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "noto-arabic".into(),
        Arc::new(FontData::from_static(include_bytes!(
            "../../assets/fonts/NotoSansArabic.ttf"
        ))),
    );
    fonts.font_data.insert(
        "inter".into(),
        Arc::new(FontData::from_static(include_bytes!(
            "../../assets/fonts/Inter-Regular.ttf"
        ))),
    );
    fonts.font_data.insert(
        "inter-semibold".into(),
        Arc::new(FontData::from_static(include_bytes!(
            "../../assets/fonts/Inter-SemiBold.ttf"
        ))),
    );
    let fallbacks: Vec<String> = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let mut regular = vec!["inter".to_string(), "noto-arabic".to_string()];
    regular.extend(fallbacks.iter().cloned());
    let mut semi = vec!["inter-semibold".to_string(), "noto-arabic".to_string()];
    semi.extend(fallbacks);
    fonts.families.insert(FontFamily::Proportional, regular);
    fonts.families.insert(semibold(), semi);
    fonts.families.insert(
        FontFamily::Name("arabic".into()),
        vec!["noto-arabic".into(), "inter".into()],
    );
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .push("noto-arabic".into());
    ctx.set_fonts(fonts);
}

pub fn apply(ctx: &egui::Context, p: &Palette) {
    let mut v = if p.dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    v.panel_fill = p.bg;
    v.window_fill = p.surf;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.extreme_bg_color = if p.dark { hex(0x0F1117) } else { hex(0xFFFFFF) };
    v.faint_bg_color = p.alt;
    v.override_text_color = Some(p.text);
    v.hyperlink_color = p.accent;
    v.selection.bg_fill = if p.dark { hex(0x4C1D95) } else { hex(0xDDD6FE) };
    v.selection.stroke = Stroke::new(1.0, p.text);
    v.window_corner_radius = CornerRadius::same(14);
    v.menu_corner_radius = CornerRadius::same(10);
    let r = CornerRadius::same(8);
    for (w, fill, stroke) in [
        (&mut v.widgets.noninteractive, p.surf, p.border),
        (&mut v.widgets.inactive, p.alt, p.border),
        (&mut v.widgets.hovered, p.hover, p.border),
        (&mut v.widgets.active, p.hover, p.accent),
        (&mut v.widgets.open, p.hover, p.border),
    ] {
        w.bg_fill = fill;
        w.weak_bg_fill = fill;
        w.bg_stroke = Stroke::new(1.0, stroke);
        w.corner_radius = r;
        w.fg_stroke = Stroke::new(1.5, p.text);
    }
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    ctx.set_visuals(v);
    ctx.global_style_mut(|s| {
        s.text_styles = [
            (TextStyle::Heading, bold(22.0)),
            (TextStyle::Body, font(13.5)),
            (TextStyle::Button, font(13.5)),
            (TextStyle::Small, font(12.0)),
            (
                TextStyle::Monospace,
                FontId::new(12.5, FontFamily::Monospace),
            ),
        ]
        .into();
        s.spacing.item_spacing = egui::vec2(8.0, 8.0);
        s.spacing.button_padding = egui::vec2(12.0, 7.0);
        s.spacing.interact_size.y = 32.0;
    });
}
