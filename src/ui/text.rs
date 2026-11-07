//! Display-only bidi runs. File names, URLs, search and filesystem paths stay logical UTF-8.
use egui::{Color32, FontFamily, FontId, TextFormat, text::LayoutJob};

pub fn contains_arabic(text: &str) -> bool {
    text.chars().any(|c| matches!(c as u32, 0x0600..=0x06ff | 0x0750..=0x077f | 0x08a0..=0x08ff | 0xfb50..=0xfdff | 0xfe70..=0xfeff))
}

pub fn job(text: &str, font: FontId, color: Color32) -> LayoutJob {
    if !contains_arabic(text) {
        return LayoutJob::simple_singleline(text.to_string(), font, color);
    }
    // File-manager containers are LTR: keep digits and the Latin extension
    // together while shaping Arabic runs right-to-left.
    // Isolate ASCII filename components (for example 2026.pdf). Otherwise the
    // bidi algorithm may place the number on the other side of an Arabic stem.
    let mut prepared = String::new();
    let mut ascii = String::new();
    let flush_ascii = |ascii: &mut String, prepared: &mut String| {
        if ascii.chars().any(|c| c.is_ascii_alphanumeric()) {
            prepared.push('\u{2066}');
            prepared.push_str(ascii);
            prepared.push('\u{2069}');
        } else {
            prepared.push_str(ascii);
        }
        ascii.clear();
    };
    for character in text.chars() {
        if character.is_ascii() {
            ascii.push(character);
        } else {
            flush_ascii(&mut ascii, &mut prepared);
            prepared.push(character);
        }
    }
    flush_ascii(&mut ascii, &mut prepared);
    let bidi = unicode_bidi::BidiInfo::new(&prepared, Some(unicode_bidi::Level::ltr()));
    let mut job = LayoutJob::default();
    for paragraph in &bidi.paragraphs {
        let (levels, runs) = bidi.visual_runs(paragraph, paragraph.range.clone());
        for run in runs {
            let content: String = prepared[run.clone()]
                .chars()
                .filter(|c| !matches!(*c, '\u{2066}' | '\u{2069}'))
                .collect();
            if content.is_empty() {
                continue;
            }
            let arabic = contains_arabic(&content);
            let selected = if arabic {
                FontId::new(font.size, FontFamily::Name("arabic".into()))
            } else {
                font.clone()
            };
            let format = TextFormat {
                font_id: selected,
                color,
                valign: egui::Align::Center,
                ..Default::default()
            };
            // egui shapes each section with HarfRust. Do not reverse Arabic letters:
            // the shaper joins and orders the RTL run itself. Even-level Arabic
            // numerals stay in logical order via separate, non-joining glyph runs.
            if arabic && levels[run.start].is_ltr() {
                for character in content.chars() {
                    job.append(&character.to_string(), 0.0, format.clone());
                }
            } else {
                job.append(&content, 0.0, format);
            }
        }
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn embedded_font_covers_arabic_and_layout_is_renderable() {
        let ctx = egui::Context::default();
        super::super::theme::install_fonts(&ctx);
        let mut frame = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.label(job(
                "تقرير العمل 2026.pdf",
                FontId::proportional(14.0),
                Color32::WHITE,
            ));
        });
        frame.textures_delta.clear();
        ctx.fonts_mut(|fonts| {
            assert!(fonts.has_glyphs(&FontId::proportional(14.0), "تقرير العمل"))
        });
    }
    #[test]
    fn mixed_file_names_keep_latin_suffix_and_original_data() {
        let name = "تقرير العمل 2026.pdf";
        let job = job(name, FontId::proportional(14.0), Color32::WHITE);
        assert!(job.text.contains("2026.pdf"));
        assert!(
            job.sections
                .iter()
                .any(|s| s.format.font_id.family == FontFamily::Name("arabic".into()))
        );
        assert_eq!(name, "تقرير العمل 2026.pdf");
    }
}
