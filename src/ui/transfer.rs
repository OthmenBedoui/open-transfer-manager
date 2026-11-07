//! "Transfer" page: copy files from one drive or server to others (Google Drive →
//! OneDrive, SFTP → S3…). rclone streams them through this computer without writing
//! them to its disk; progress shows in the same Transfers list as uploads.

use super::app::{OtmApp, View};
use super::icons::{self, Icon};
use super::send::{engine_banner, mono_badge};
use super::theme::{self, Palette};
use super::widgets;
use crate::model::fmt_size;
use crate::rclone::Entry;
use crate::uploads::{self, Uploads};
use egui::{Color32, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use std::collections::BTreeSet;
use std::sync::mpsc;

type Listing = Result<Vec<Entry>, String>;

#[derive(Default)]
pub struct TransferState {
    pub source: Option<u64>,
    pub path: String,
    pub entries: Vec<Entry>,
    pub picked: BTreeSet<String>,
    pub error: Option<String>,
    pub chosen: BTreeSet<u64>,
    pub links: bool,
    loading: Option<mpsc::Receiver<Listing>>,
}

impl TransferState {
    pub fn for_demo(source: u64) -> Self {
        let e = |name: &str, is_dir: bool, size: Option<u64>| Entry {
            name: name.into(),
            is_dir,
            size,
        };
        TransferState {
            source: Some(source),
            path: "Projects/OTM".into(),
            entries: vec![
                e("Design", true, None),
                e("Releases", true, None),
                e("demo-video-final.mp4", false, Some(248_000_000)),
                e("cover.png", false, Some(1_200_000)),
                e("notes.pdf", false, Some(312_000)),
            ],
            picked: ["Releases".to_string(), "demo-video-final.mp4".to_string()].into(),
            chosen: [3].into(),
            links: true,
            ..Default::default()
        }
    }

    fn open(&mut self, uploads: &Uploads, path: String) {
        self.path = path;
        self.picked.clear();
        self.entries.clear();
        self.error = None;
        self.loading = self
            .source
            .and_then(|id| uploads.connector(id))
            .map(|c| Uploads::browse(c, self.path.clone()));
    }

    fn item_path(&self, name: &str) -> String {
        if self.path.is_empty() {
            name.to_string()
        } else {
            format!("{}/{name}", self.path.trim_end_matches('/'))
        }
    }
}

impl OtmApp {
    pub(super) fn transfer_view(&mut self, ui: &mut egui::Ui, p: &Palette) {
        if let Some(rx) = &self.transfer.loading
            && let Ok(r) = rx.try_recv()
        {
            self.transfer.loading = None;
            match r {
                Ok(v) => self.transfer.entries = v,
                Err(e) => self.transfer.error = Some(e),
            }
        }
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.label(egui::RichText::new("Transfer between drives").font(theme::bold(22.0)));
                ui.label(
                    egui::RichText::new("Copy files from one drive or server to others — Google Drive to OneDrive, SFTP to S3…")
                        .size(12.0)
                        .color(p.muted),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::secondary(ui, p, "Add destination", Some(Icon::Plus)).clicked() {
                    self.view = View::Connectors;
                }
            });
        });
        ui.add_space(10.0);
        if let Some(r) = engine_banner(ui, p, &mut self.send.install) {
            match r {
                Ok(t) => self.toast(t),
                Err(e) => self.toast(e),
            }
        }
        let w = ui.available_width();
        let right_w = (w * 0.4).clamp(280.0, 360.0);
        let top_h = 360.0_f32.min(ui.available_height() * 0.62).max(240.0);
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                vec2(w - right_w - 14.0, top_h),
                egui::Layout::top_down(egui::Align::Min),
                |ui| self.transfer_source(ui, p, top_h),
            );
            ui.add_space(14.0);
            ui.allocate_ui_with_layout(
                vec2(right_w, top_h),
                egui::Layout::top_down(egui::Align::Min),
                |ui| self.transfer_targets(ui, p, top_h),
            );
        });
        ui.add_space(12.0);
        let jobs = self.uploads.jobs();
        self.transfers(ui, p, &jobs);
    }

    fn transfer_source(&mut self, ui: &mut egui::Ui, p: &Palette, h: f32) {
        let conns = self.uploads.connectors();
        if self
            .transfer
            .source
            .is_some_and(|s| !conns.iter().any(|c| c.id == s))
        {
            self.transfer = Default::default();
        }
        widgets::card(p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(h - 28.0);
            ui.label(egui::RichText::new("From").font(theme::bold(14.0)));
            ui.add_space(4.0);
            if conns.len() < 2 {
                ui.label(
                    egui::RichText::new("Connect at least two drives or servers (for example Google Drive and OneDrive) to move files between them.")
                        .size(12.5)
                        .color(p.muted),
                );
                ui.add_space(8.0);
                if widgets::primary(ui, p, "Connect a drive", Some(Icon::Plug)).clicked() {
                    self.view = View::Connectors;
                }
                return;
            }
            // source picker: one chip per destination, with its logo
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                for c in &conns {
                    let on = self.transfer.source == Some(c.id);
                    let name = c.display_name();
                    let g = ui.painter().layout_no_wrap(name.clone(), theme::font(12.5), p.text);
                    let (r, resp) = ui.allocate_exact_size(vec2(g.size().x + 44.0, 32.0), Sense::click());
                    let (bg, border) = if on {
                        (theme::tint(p.btn, if p.dark { 0.18 } else { 0.10 }), p.btn)
                    } else if resp.hovered() {
                        (p.hover, p.border)
                    } else {
                        (Color32::TRANSPARENT, p.border)
                    };
                    ui.painter().rect_filled(r, 16, bg);
                    ui.painter().rect_stroke(r, 16, Stroke::new(1.0, border), StrokeKind::Inside);
                    mono_badge(ui.painter(), Rect::from_min_size(r.min + vec2(5.0, 5.0), vec2(22.0, 22.0)), "", c.kind, p.dark);
                    ui.painter().galley(pos2(r.min.x + 34.0, r.center().y - g.size().y / 2.0), g, p.text);
                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && !on {
                        self.transfer.source = Some(c.id);
                        self.transfer.chosen.remove(&c.id);
                        let root = c.browse_root();
                        self.transfer.open(&self.uploads, root);
                    }
                }
            });
            let Some(src) = self.transfer.source.and_then(|id| self.uploads.connector(id)) else {
                ui.add_space(10.0);
                ui.label(egui::RichText::new("Pick where the files are.").size(12.5).color(p.muted));
                return;
            };
            ui.add_space(8.0);
            // breadcrumb
            ui.horizontal(|ui| {
                let at_root = self.transfer.path.is_empty();
                if ui
                    .add_enabled_ui(!at_root, |ui| widgets::icon_button(ui, p, Icon::Upload, "Parent folder", !at_root))
                    .inner
                    .clicked()
                {
                    let parent = self.transfer.path.rsplit_once('/').map(|(a, _)| a.to_string()).unwrap_or_default();
                    self.transfer.open(&self.uploads, parent);
                }
                let shown = format!("{} / {}", src.display_name(), self.transfer.path.replace('/', " / "));
                ui.add(egui::Label::new(egui::RichText::new(shown.trim_end_matches(" / ")).size(12.5).color(p.muted)).truncate());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(egui::Label::new(egui::RichText::new("Refresh").size(12.0).color(p.accent)).sense(Sense::click())).clicked() {
                        let path = self.transfer.path.clone();
                        self.transfer.open(&self.uploads, path);
                    }
                });
            });
            ui.add_space(4.0);
            if self.transfer.loading.is_some() {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new());
                    ui.label(egui::RichText::new("Reading the folder…").size(12.5).color(p.muted));
                });
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
                return;
            }
            if let Some(e) = &self.transfer.error {
                ui.label(egui::RichText::new(e).size(12.5).color(p.danger));
                return;
            }
            if self.transfer.entries.is_empty() {
                ui.label(egui::RichText::new("This folder is empty.").size(12.5).color(p.muted));
                return;
            }
            let list_h = (ui.available_height() - 4.0).max(60.0);
            let mut open = None;
            egui::ScrollArea::vertical()
                .id_salt("transfer-files")
                .max_height(list_h)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for e in &self.transfer.entries {
                        let on = self.transfer.picked.contains(&e.name);
                        let (row, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::click());
                        if on {
                            ui.painter().rect_filled(row, 8, theme::tint(p.btn, if p.dark { 0.12 } else { 0.07 }));
                        } else if resp.hovered() {
                            ui.painter().rect_filled(row, 8, p.hover);
                        }
                        let bx = Rect::from_center_size(pos2(row.min.x + 14.0, row.center().y), vec2(16.0, 16.0));
                        if on {
                            ui.painter().rect_filled(bx, 4, p.btn);
                            icons::paint(ui.painter(), bx.shrink(2.5), Icon::Check, Color32::WHITE);
                        } else {
                            ui.painter().rect_stroke(bx, 4, Stroke::new(1.4, p.muted), StrokeKind::Inside);
                        }
                        let badge = Rect::from_min_size(pos2(row.min.x + 30.0, row.center().y - 13.0), vec2(26.0, 26.0));
                        if e.is_dir {
                            ui.painter().rect_filled(badge, 7, theme::tint(p.warn, 0.16));
                            icons::paint(ui.painter(), badge.shrink(6.0), Icon::Folder, p.warn);
                        } else {
                            widgets::type_badge(ui.painter(), badge, &e.name, p.dark);
                        }
                        let g = widgets::elided(ui.painter(), &e.name, theme::font(13.0), p.text, row.width() - 190.0);
                        ui.painter().galley(pos2(row.min.x + 64.0, row.center().y - g.size().y / 2.0), g, p.text);
                        if e.is_dir {
                            let ob = Rect::from_center_size(pos2(row.max.x - 30.0, row.center().y), vec2(52.0, 24.0));
                            let or = ui.interact(ob, ui.scope_id().with(("open", &e.name)), Sense::click());
                            ui.painter().rect_filled(ob, 6, if or.hovered() { p.alt } else { Color32::TRANSPARENT });
                            ui.painter().text(ob.center(), egui::Align2::CENTER_CENTER, "Open ›", theme::font(12.0), p.accent);
                            if or.clicked() {
                                open = Some(self.transfer.item_path(&e.name));
                            }
                        } else {
                            ui.painter().text(
                                pos2(row.max.x - 10.0, row.center().y),
                                egui::Align2::RIGHT_CENTER,
                                fmt_size(e.size),
                                theme::font(12.0),
                                p.muted,
                            );
                        }
                        if resp.double_clicked() && e.is_dir {
                            open = Some(self.transfer.item_path(&e.name));
                        } else if resp.clicked() {
                            if on {
                                self.transfer.picked.remove(&e.name);
                            } else {
                                self.transfer.picked.insert(e.name.clone());
                            }
                        }
                    }
                });
            if let Some(path) = open {
                self.transfer.open(&self.uploads, path);
            }
        });
    }

    fn transfer_targets(&mut self, ui: &mut egui::Ui, p: &Palette, h: f32) {
        let source = self.transfer.source;
        let conns: Vec<_> = self
            .uploads
            .connectors()
            .into_iter()
            .filter(|c| Some(c.id) != source)
            .collect();
        self.transfer
            .chosen
            .retain(|id| conns.iter().any(|c| c.id == *id));
        widgets::card(p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(h - 28.0);
            ui.label(egui::RichText::new("To").font(theme::bold(14.0)));
            ui.add_space(4.0);
            let list_h = (ui.available_height() - 140.0).max(60.0);
            egui::ScrollArea::vertical()
                .id_salt("transfer-dests")
                .max_height(list_h)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for c in &conns {
                        let on = self.transfer.chosen.contains(&c.id);
                        let (row, resp) = ui
                            .allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::click());
                        let (bg, border) = if on {
                            (theme::tint(p.btn, if p.dark { 0.14 } else { 0.08 }), p.btn)
                        } else if resp.hovered() {
                            (p.hover, p.border)
                        } else {
                            (Color32::TRANSPARENT, p.border)
                        };
                        ui.painter().rect_filled(row, 10, bg);
                        ui.painter().rect_stroke(
                            row,
                            10,
                            Stroke::new(1.0, border),
                            StrokeKind::Inside,
                        );
                        mono_badge(
                            ui.painter(),
                            Rect::from_min_size(
                                pos2(row.min.x + 8.0, row.center().y - 15.0),
                                vec2(30.0, 30.0),
                            ),
                            "",
                            c.kind,
                            p.dark,
                        );
                        let tw = row.width() - 80.0;
                        let g = widgets::elided(
                            ui.painter(),
                            &c.display_name(),
                            theme::bold(13.0),
                            p.text,
                            tw,
                        );
                        ui.painter().galley(
                            pos2(row.min.x + 46.0, row.center().y - 16.0),
                            g,
                            p.text,
                        );
                        let g = widgets::elided(
                            ui.painter(),
                            &c.address(),
                            theme::font(11.5),
                            p.muted,
                            tw,
                        );
                        ui.painter().galley(
                            pos2(row.min.x + 46.0, row.center().y + 1.0),
                            g,
                            p.muted,
                        );
                        let bx = Rect::from_center_size(
                            pos2(row.max.x - 18.0, row.center().y),
                            vec2(18.0, 18.0),
                        );
                        if on {
                            ui.painter().rect_filled(bx, 5, p.btn);
                            icons::paint(ui.painter(), bx.shrink(3.0), Icon::Check, Color32::WHITE);
                        } else {
                            ui.painter().rect_stroke(
                                bx,
                                5,
                                Stroke::new(1.5, p.muted),
                                StrokeKind::Inside,
                            );
                        }
                        if resp
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .clicked()
                        {
                            if on {
                                self.transfer.chosen.remove(&c.id);
                            } else {
                                self.transfer.chosen.insert(c.id);
                            }
                        }
                        ui.add_space(4.0);
                    }
                });
            ui.add_space(4.0);
            let mut links = self.transfer.links;
            if widgets::switch_row(
                ui,
                p,
                "Create share links",
                "Where the destination supports it",
                &mut links,
            ) {
                self.transfer.links = links;
            }
            ui.add_space(6.0);
            let (n, m) = (self.transfer.picked.len(), self.transfer.chosen.len());
            let label = if n == 0 {
                "Tick files or folders to copy".to_string()
            } else if m == 0 {
                "Choose where to copy them".to_string()
            } else {
                format!(
                    "Copy {n} item{} to {m} destination{}",
                    if n == 1 { "" } else { "s" },
                    if m == 1 { "" } else { "s" }
                )
            };
            let ready = n > 0 && m > 0 && uploads::engine_ready();
            let clicked = ui
                .add_enabled_ui(ready, |ui| {
                    ui.set_width(ui.available_width());
                    widgets::primary(ui, p, &label, Some(Icon::Swap)).clicked()
                })
                .inner;
            if clicked && let Some(src) = self.transfer.source {
                let items: Vec<(String, bool, Option<u64>)> = self
                    .transfer
                    .entries
                    .iter()
                    .filter(|e| self.transfer.picked.contains(&e.name))
                    .map(|e| (self.transfer.item_path(&e.name), e.is_dir, e.size))
                    .collect();
                let dests: Vec<u64> = self.transfer.chosen.iter().copied().collect();
                let added = self
                    .uploads
                    .send_remote(src, &items, &dests, self.transfer.links);
                self.transfer.picked.clear();
                self.toast(format!(
                    "{added} transfer{} started",
                    if added == 1 { "" } else { "s" }
                ));
            }
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(
                    "Files stream through this computer; nothing is saved on its disk.",
                )
                .size(11.5)
                .color(p.muted),
            );
        });
    }
}
