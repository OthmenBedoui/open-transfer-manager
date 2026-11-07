//! Destination tiles: cloud drives show their own logo, servers a symbolic icon.
//!
//! Logos (Google Drive, Dropbox, MEGA, Box) come from Simple Icons (CC0) and are only used
//! to name the service the user connects to; the trademarks belong to their owners.
//! Microsoft (OneDrive) and pCloud have no logo there, so they get a cloud in their colour.

use super::icons::{self, Icon};
use super::send::kind_colors;
use super::theme;
use crate::connectors::Kind;
use egui::{Color32, Painter, Rect, TextureHandle, pos2, vec2};
use std::collections::HashMap;
use std::sync::Mutex;

fn logo_bytes(kind: Kind) -> Option<(&'static str, &'static [u8])> {
    Some(match kind {
        Kind::Gdrive => (
            "googledrive",
            include_bytes!("../../assets/brands/googledrive.png"),
        ),
        Kind::Dropbox => ("dropbox", include_bytes!("../../assets/brands/dropbox.png")),
        Kind::Mega => ("mega", include_bytes!("../../assets/brands/mega.png")),
        Kind::Box => ("box", include_bytes!("../../assets/brands/box.png")),
        _ => return None,
    })
}

/// The brand colour of a cloud drive.
pub fn brand_color(kind: Kind) -> Option<Color32> {
    Some(match kind {
        Kind::Gdrive => Color32::from_rgb(0x42, 0x85, 0xF4),
        Kind::Onedrive => Color32::from_rgb(0x00, 0x78, 0xD4),
        Kind::Dropbox => Color32::from_rgb(0x00, 0x61, 0xFF),
        Kind::Mega => Color32::from_rgb(0xD9, 0x27, 0x2E),
        Kind::Pcloud => Color32::from_rgb(0x17, 0xBE, 0xD0),
        Kind::Box => Color32::from_rgb(0x00, 0x61, 0xD5),
        _ => return None,
    })
}

static TEXTURES: Mutex<Option<HashMap<&'static str, TextureHandle>>> = Mutex::new(None);

fn logo(painter: &Painter, kind: Kind) -> Option<TextureHandle> {
    let (name, bytes) = logo_bytes(kind)?;
    let mut guard = TEXTURES.lock().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(HashMap::new);
    if let Some(t) = map.get(name) {
        return Some(t.clone());
    }
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    let ci = egui::ColorImage::from_rgba_unmultiplied(
        [img.width() as usize, img.height() as usize],
        img.as_raw(),
    );
    let t = painter
        .ctx()
        .load_texture(format!("brand-{name}"), ci, egui::TextureOptions::LINEAR);
    map.insert(name, t.clone());
    Some(t)
}

fn server_icon(kind: Kind) -> Icon {
    match kind {
        Kind::Sftp => Icon::Terminal,
        Kind::Ftp => Icon::Server,
        Kind::Webdav => Icon::Globe,
        Kind::S3 => Icon::Bucket,
        _ => Icon::Cloud,
    }
}

pub fn dest_icon(painter: &Painter, rect: Rect, kind: Kind, dark: bool) {
    let radius = (rect.width() * 0.27) as u8;
    if let Some(color) = brand_color(kind) {
        // white tile in both themes: logos keep their real colours
        let bg = if dark {
            Color32::from_rgb(0xF4, 0xF6, 0xFA)
        } else {
            theme::tint(color, 0.10)
        };
        painter.rect_filled(rect, radius, bg);
        let inner = Rect::from_center_size(rect.center(), rect.size() * 0.62);
        match logo(painter, kind) {
            Some(t) => {
                painter.image(
                    t.id(),
                    inner,
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            None => {
                icons::paint(
                    painter,
                    inner.expand(rect.width() * 0.04),
                    Icon::Cloud,
                    color,
                );
                if kind == Kind::Pcloud {
                    painter.text(
                        inner.center() + vec2(0.0, rect.height() * 0.05),
                        egui::Align2::CENTER_CENTER,
                        "p",
                        theme::bold(rect.width() * 0.28),
                        color,
                    );
                }
            }
        }
        return;
    }
    let (bg, fg) = kind_colors(kind, dark);
    painter.rect_filled(rect, radius, bg);
    icons::paint(
        painter,
        Rect::from_center_size(rect.center(), rect.size() * 0.6),
        server_icon(kind),
        fg,
    );
}
