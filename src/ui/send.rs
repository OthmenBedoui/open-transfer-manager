//! "Send" page: drop files, tick destinations, follow each upload.

use super::app::{OtmApp, View};
use super::icons::{self, Icon};
use super::theme::{self, Palette};
use super::widgets;
use crate::connectors::Kind;
use crate::model::{fmt_eta, fmt_rate, fmt_size};
use crate::uploads::{self, Job, JobStatus};
use egui::{Align2, Color32, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::mpsc;

#[derive(Default)]
pub struct SendState {
    pub files: Vec<(PathBuf, u64)>,
    pub chosen: BTreeSet<u64>,
    pub links: bool,
    pub install: Option<mpsc::Receiver<Result<String, String>>>,
}

impl SendState {
    pub fn add(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        for p in paths {
            if !self.files.iter().any(|(f, _)| *f == p) && p.exists() {
                let size = uploads::size_of(&p);
                self.files.push((p, size));
            }
        }
    }
}

/// Background / foreground colours of a destination's two-letter badge.
pub fn kind_colors(kind: Kind, dark: bool) -> (Color32, Color32) {
    let rgb = |c: (u8, u8, u8)| Color32::from_rgb(c.0, c.1, c.2);
    // (dark background, dark foreground, light foreground)
    let (bg, fg, light_fg) = match kind {
        Kind::Sftp => ((0x2A, 0x2F, 0x3B), (0xCB, 0xD5, 0xE1), (0x47, 0x55, 0x69)),
        Kind::Ftp => ((0x16, 0x30, 0x4F), (0x60, 0xA5, 0xFA), (0x1D, 0x4E, 0xD8)),
        Kind::Webdav => ((0x12, 0x38, 0x32), (0x2D, 0xD4, 0xBF), (0x0F, 0x76, 0x6E)),
        Kind::S3 => ((0x3D, 0x2A, 0x10), (0xFB, 0xBF, 0x24), (0xB4, 0x53, 0x09)),
        Kind::Gdrive => ((0x1C, 0x3D, 0x2E), (0x4A, 0xDE, 0x80), (0x15, 0x80, 0x3D)),
        Kind::Onedrive => ((0x17, 0x25, 0x54), (0x93, 0xC5, 0xFD), (0x1E, 0x40, 0xAF)),
        Kind::Dropbox => ((0x1E, 0x29, 0x4F), (0x81, 0x8C, 0xF8), (0x43, 0x38, 0xCA)),
        Kind::Mega => ((0x4A, 0x1C, 0x22), (0xF8, 0x71, 0x71), (0xB9, 0x1C, 0x1C)),
        Kind::Pcloud => ((0x13, 0x2E, 0x45), (0x38, 0xBD, 0xF8), (0x03, 0x69, 0xA1)),
        Kind::Box => ((0x10, 0x33, 0x4A), (0x7D, 0xD3, 0xFC), (0x07, 0x59, 0x85)),
    };
    if dark {
        (rgb(bg), rgb(fg))
    } else {
        (theme::tint(rgb(light_fg), 0.12), rgb(light_fg))
    }
}

pub fn mono_badge(painter: &egui::Painter, rect: Rect, text: &str, kind: Kind, dark: bool) {
    let (bg, fg) = kind_colors(kind, dark);
    painter.rect_filled(rect, (rect.width() * 0.27) as u8, bg);
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        theme::bold(rect.width() * 0.33),
        fg,
    );
}

/// Banner shown on Send and Connectors while rclone is missing.
pub fn engine_banner(
    ui: &mut egui::Ui,
    p: &Palette,
    install: &mut Option<mpsc::Receiver<Result<String, String>>>,
) -> Option<Result<String, String>> {
    let mut done = None;
    if let Some(rx) = install
        && let Ok(r) = rx.try_recv()
    {
        done = Some(r);
        *install = None;
    }
    if uploads::engine_ready() {
        return done;
    }
    widgets::card(p)
        .fill(theme::tint(p.warn, if p.dark { 0.10 } else { 0.08 }))
        .stroke(Stroke::new(1.0, theme::tint(p.warn, 0.45)))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.label(egui::RichText::new("The upload engine isn't installed yet").font(theme::bold(13.5)));
                    ui.label(
                        egui::RichText::new(
                            "OTM sends files with rclone (open source, MIT). It downloads the official build and checks its checksum.",
                        )
                        .size(12.0)
                        .color(p.muted),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if install.is_some() {
                        ui.add(egui::Spinner::new());
                        ui.label(egui::RichText::new("Installing…").size(12.5).color(p.muted));
                    } else if widgets::primary(ui, p, "Install rclone", Some(Icon::Download)).clicked() {
                        let (tx, rx) = mpsc::channel();
                        let ctx = ui.ctx().clone();
                        std::thread::spawn(move || {
                            let _ = tx.send(crate::rclone::install_or_update());
                            ctx.request_repaint();
                        });
                        *install = Some(rx);
                    }
                });
            });
        });
    ui.add_space(10.0);
    done
}

impl OtmApp {
    pub(super) fn send_view(&mut self, ui: &mut egui::Ui, p: &Palette) {
        let jobs = self.uploads.jobs();
        let running = jobs
            .iter()
            .filter(|j| j.status == JobStatus::Running)
            .count();
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.label(egui::RichText::new("Send").font(theme::bold(22.0)));
                let mut sub = "Drop files, pick destinations, send everywhere at once.".to_string();
                if running > 0 {
                    sub = format!(
                        "{running} uploading · ↑ {}",
                        fmt_rate(Some(self.uploads.total_rate()))
                    );
                }
                ui.label(egui::RichText::new(sub).size(12.0).color(p.muted));
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
        let top_h = 330.0_f32.min(ui.available_height() * 0.6).max(220.0);
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                vec2(w - right_w - 14.0, top_h),
                egui::Layout::top_down(egui::Align::Min),
                |ui| self.drop_and_files(ui, p, top_h),
            );
            ui.add_space(14.0);
            ui.allocate_ui_with_layout(
                vec2(right_w, top_h),
                egui::Layout::top_down(egui::Align::Min),
                |ui| self.destinations(ui, p, top_h),
            );
        });
        ui.add_space(12.0);
        self.transfers(ui, p, &jobs);
    }

    fn drop_and_files(&mut self, ui: &mut egui::Ui, p: &Palette, h: f32) {
        let hovering = ui.ctx().input(|i| !i.raw.hovered_files.is_empty());
        let zone_h = if self.send.files.is_empty() { h } else { 104.0 };
        let (rect, resp) =
            ui.allocate_exact_size(vec2(ui.available_width(), zone_h), Sense::click());
        let accent = if hovering || resp.hovered() {
            p.btn
        } else {
            theme::tint(p.btn, 0.55)
        };
        ui.painter().rect_filled(
            rect,
            14,
            theme::tint(
                p.btn,
                if hovering {
                    0.16
                } else if p.dark {
                    0.06
                } else {
                    0.04
                },
            ),
        );
        let r = rect.shrink(1.0);
        let pts = [
            r.left_top(),
            r.right_top(),
            r.right_bottom(),
            r.left_bottom(),
            r.left_top(),
        ];
        ui.painter().extend(egui::Shape::dashed_line(
            &pts,
            Stroke::new(1.6, accent),
            7.0,
            5.0,
        ));
        let c = rect.center();
        let big = self.send.files.is_empty();
        let icon_y = if big { c.y - 26.0 } else { c.y - 18.0 };
        icons::paint(
            ui.painter(),
            Rect::from_center_size(pos2(c.x, icon_y), vec2(28.0, 28.0)),
            Icon::Upload,
            p.accent,
        );
        ui.painter().text(
            pos2(c.x, icon_y + 26.0),
            Align2::CENTER_CENTER,
            if hovering {
                "Drop to add"
            } else {
                "Drop files or folders here"
            },
            theme::bold(13.5),
            p.text,
        );
        ui.painter().text(
            pos2(c.x, icon_y + 44.0),
            Align2::CENTER_CENTER,
            "or click to browse · also “Send to…” on a finished download",
            theme::font(12.0),
            p.muted,
        );
        if resp
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
            && let Some(files) = rfd::FileDialog::new().pick_files()
        {
            self.send.add(files);
        }
        if self.send.files.is_empty() {
            return;
        }
        ui.add_space(8.0);
        let mut remove = None;
        widgets::card(p)
            .inner_margin(egui::Margin::symmetric(10, 6))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                let list_h = (ui.available_height() - 30.0).max(40.0);
                egui::ScrollArea::vertical()
                    .id_salt("send-files")
                    .max_height(list_h)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        for (i, (path, size)) in self.send.files.iter().enumerate() {
                            let (row, _) = ui.allocate_exact_size(
                                vec2(ui.available_width(), 38.0),
                                Sense::hover(),
                            );
                            let name = path
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_else(|| path.display().to_string());
                            let badge = Rect::from_min_size(
                                pos2(row.min.x, row.center().y - 15.0),
                                vec2(30.0, 30.0),
                            );
                            if path.is_dir() {
                                ui.painter()
                                    .rect_filled(badge, 8, theme::tint(p.warn, 0.16));
                                icons::paint(ui.painter(), badge.shrink(7.0), Icon::Folder, p.warn);
                            } else {
                                widgets::type_badge(ui.painter(), badge, &name, p.dark);
                            }
                            let text_w = row.width() - 150.0;
                            let g = widgets::elided(
                                ui.painter(),
                                &name,
                                theme::font(13.0),
                                p.text,
                                text_w,
                            );
                            ui.painter().galley(
                                pos2(row.min.x + 40.0, row.center().y - 15.0),
                                g,
                                p.text,
                            );
                            ui.painter().text(
                                pos2(row.min.x + 40.0, row.center().y + 8.0),
                                Align2::LEFT_CENTER,
                                if path.is_dir() {
                                    format!("Folder · {}", fmt_size(Some(*size)))
                                } else {
                                    fmt_size(Some(*size))
                                },
                                theme::font(11.5),
                                p.muted,
                            );
                            let x = Rect::from_center_size(
                                pos2(row.max.x - 16.0, row.center().y),
                                vec2(28.0, 28.0),
                            );
                            let xr = ui.interact(x, ui.scope_id().with(("rm", i)), Sense::click());
                            if xr.hovered() {
                                ui.painter().rect_filled(x, 7, p.hover);
                            }
                            icons::paint(ui.painter(), x.shrink(8.0), Icon::Close, p.muted);
                            if xr.on_hover_text("Remove from the list").clicked() {
                                remove = Some(i);
                            }
                        }
                    });
                let total: u64 = self.send.files.iter().map(|(_, s)| s).sum();
                let n = self.send.files.len();
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!(
                            "{n} item{} · {}",
                            if n == 1 { "" } else { "s" },
                            fmt_size(Some(total))
                        ))
                        .size(12.0)
                        .color(p.muted),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Label::new(
                                    egui::RichText::new("Clear").size(12.0).color(p.accent),
                                )
                                .sense(Sense::click()),
                            )
                            .clicked()
                        {
                            self.send.files.clear();
                        }
                    });
                });
            });
        if let Some(i) = remove {
            self.send.files.remove(i);
        }
    }

    fn destinations(&mut self, ui: &mut egui::Ui, p: &Palette, h: f32) {
        let conns = self.uploads.connectors();
        self.send
            .chosen
            .retain(|id| conns.iter().any(|c| c.id == *id));
        widgets::card(p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(h - 28.0);
            ui.label(egui::RichText::new("Send to").font(theme::bold(14.0)));
            ui.add_space(4.0);
            if conns.is_empty() {
                ui.label(
                    egui::RichText::new("No destinations yet. Connect a drive (Google Drive, OneDrive, Dropbox…) or a server once, then pick it here.")
                        .size(12.5)
                        .color(p.muted),
                );
                ui.add_space(8.0);
                if widgets::primary(ui, p, "Add a destination", Some(Icon::Plug)).clicked() {
                    self.view = View::Connectors;
                }
                return;
            }
            let list_h = (ui.available_height() - 96.0).max(60.0);
            egui::ScrollArea::vertical()
                .id_salt("send-dests")
                .max_height(list_h)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for c in &conns {
                        let on = self.send.chosen.contains(&c.id);
                        let (row, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::click());
                        let (bg, border) = if on {
                            (theme::tint(p.btn, if p.dark { 0.14 } else { 0.08 }), p.btn)
                        } else if resp.hovered() {
                            (p.hover, p.border)
                        } else {
                            (Color32::TRANSPARENT, p.border)
                        };
                        ui.painter().rect_filled(row, 10, bg);
                        ui.painter().rect_stroke(row, 10, Stroke::new(1.0, border), StrokeKind::Inside);
                        let badge = Rect::from_min_size(pos2(row.min.x + 8.0, row.center().y - 15.0), vec2(30.0, 30.0));
                        mono_badge(ui.painter(), badge, c.kind.mono(), c.kind, p.dark);
                        let tw = row.width() - 80.0;
                        let g = widgets::elided(ui.painter(), &c.display_name(), theme::bold(13.0), p.text, tw);
                        ui.painter().galley(pos2(row.min.x + 46.0, row.center().y - 16.0), g, p.text);
                        let g = widgets::elided(ui.painter(), &c.address(), theme::font(11.5), p.muted, tw);
                        ui.painter().galley(pos2(row.min.x + 46.0, row.center().y + 1.0), g, p.muted);
                        let bx = Rect::from_center_size(pos2(row.max.x - 18.0, row.center().y), vec2(18.0, 18.0));
                        if on {
                            ui.painter().rect_filled(bx, 5, p.btn);
                            icons::paint(ui.painter(), bx.shrink(3.0), Icon::Check, Color32::WHITE);
                        } else {
                            ui.painter().rect_stroke(bx, 5, Stroke::new(1.5, p.muted), StrokeKind::Inside);
                        }
                        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            if on {
                                self.send.chosen.remove(&c.id);
                            } else {
                                self.send.chosen.insert(c.id);
                            }
                        }
                        ui.add_space(4.0);
                    }
                });
            ui.add_space(4.0);
            let mut links = self.send.links;
            if widgets::switch_row(ui, p, "Create share links", "Where the destination supports it (drives, S3, Nextcloud…)", &mut links) {
                self.send.links = links;
            }
            ui.add_space(6.0);
            let n = self.send.chosen.len();
            let ready = n > 0 && !self.send.files.is_empty() && uploads::engine_ready();
            let label = if self.send.files.is_empty() {
                "Add files to send".to_string()
            } else if n == 0 {
                "Choose a destination".to_string()
            } else {
                format!("Send to {n} destination{}", if n == 1 { "" } else { "s" })
            };
            let clicked = ui
                .add_enabled_ui(ready, |ui| {
                    ui.set_width(ui.available_width());
                    widgets::primary(ui, p, &label, Some(Icon::Upload)).clicked()
                })
                .inner;
            if clicked {
                let files: Vec<PathBuf> = self.send.files.drain(..).map(|(f, _)| f).collect();
                let dests: Vec<u64> = self.send.chosen.iter().copied().collect();
                let added = self.uploads.send(&files, &dests, self.send.links);
                self.toast(format!("{added} upload{} started", if added == 1 { "" } else { "s" }));
            }
        });
    }

    fn transfers(&mut self, ui: &mut egui::Ui, p: &Palette, jobs: &[Job]) {
        widgets::card(p)
            .inner_margin(egui::Margin::symmetric(14, 10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.set_min_height(ui.available_height() - 22.0);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Transfers").font(theme::bold(14.0)));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if jobs.iter().any(|j| j.finished())
                            && ui
                                .add(
                                    egui::Label::new(
                                        egui::RichText::new("Clear finished")
                                            .size(12.0)
                                            .color(p.accent),
                                    )
                                    .sense(Sense::click()),
                                )
                                .clicked()
                        {
                            self.uploads.clear_finished();
                        }
                    });
                });
                ui.add_space(4.0);
                if jobs.is_empty() {
                    ui.add_space(18.0);
                    ui.vertical_centered(|ui| {
                        ui.label(
                            egui::RichText::new(
                                "Nothing sent yet. Uploads appear here with their progress.",
                            )
                            .size(12.5)
                            .color(p.muted),
                        );
                    });
                    return;
                }
                egui::ScrollArea::vertical()
                    .id_salt("send-jobs")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for j in jobs.iter().rev() {
                            self.job_row(ui, p, j);
                        }
                    });
            });
    }

    fn job_row(&mut self, ui: &mut egui::Ui, p: &Palette, j: &Job) {
        let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::hover());
        ui.painter().line_segment(
            [pos2(row.min.x, row.max.y), pos2(row.max.x, row.max.y)],
            Stroke::new(1.0, p.border),
        );
        let kind = self
            .uploads
            .connector(j.dest)
            .map(|c| c.kind)
            .unwrap_or_default();
        let badge = Rect::from_min_size(pos2(row.min.x, row.center().y - 13.0), vec2(26.0, 26.0));
        mono_badge(ui.painter(), badge, j.dest_mono, kind, p.dark);

        let act_w = 116.0;
        let pill_w = 92.0;
        let meta_w = 150.0;
        let bar_w = (row.width() * 0.2).clamp(70.0, 200.0);
        let right = act_w + pill_w + meta_w + bar_w + 36.0;
        let name_w = (row.width() - right - 36.0).max(60.0);
        let x0 = row.min.x + 36.0;
        let g = widgets::elided(ui.painter(), &j.name, theme::font(13.0), p.text, name_w);
        ui.painter()
            .galley(pos2(x0, row.center().y - 16.0), g, p.text);
        let g = widgets::elided(
            ui.painter(),
            &format!("→ {}", j.dest_name),
            theme::font(11.5),
            p.muted,
            name_w,
        );
        ui.painter()
            .galley(pos2(x0, row.center().y + 1.0), g, p.muted);

        let bx = row.max.x - right + 12.0;
        let bar = Rect::from_min_size(pos2(bx, row.center().y - 3.0), vec2(bar_w, 6.0));
        let fill = match j.status {
            JobStatus::Running | JobStatus::Waiting => None,
            JobStatus::Done => Some(p.success),
            JobStatus::Failed => Some(p.danger),
            JobStatus::Stopped => Some(p.muted),
        };
        widgets::progress_bar(ui.painter(), bar, j.progress(), p.track, fill);

        let meta = match j.status {
            JobStatus::Running if j.total > 0 => format!(
                "{:.0}% · {} · {}",
                j.progress() * 100.0,
                fmt_rate(Some(j.speed)),
                fmt_eta(j.eta)
            ),
            JobStatus::Running => "Connecting…".into(),
            JobStatus::Waiting => "Waiting".into(),
            JobStatus::Done if j.is_dir => format!("Folder · {}", fmt_size(Some(j.total))),
            JobStatus::Done => fmt_size(Some(j.total)),
            JobStatus::Failed | JobStatus::Stopped => j.error.clone(),
        };
        let mx = bar.max.x + 12.0;
        let g = widgets::elided(ui.painter(), &meta, theme::font(12.0), p.muted, meta_w);
        let meta_rect = Rect::from_min_size(pos2(mx, row.center().y - 10.0), vec2(meta_w, 20.0));
        ui.painter()
            .galley(pos2(mx, row.center().y - g.size().y / 2.0), g, p.muted);
        if matches!(j.status, JobStatus::Failed) {
            ui.interact(meta_rect, ui.scope_id().with(("err", j.id)), Sense::hover())
                .on_hover_text(&j.error);
        }

        let (text, fg) = match j.status {
            JobStatus::Running => ("Uploading", p.accent),
            JobStatus::Waiting => ("Waiting", p.muted),
            JobStatus::Done => ("Done", p.success),
            JobStatus::Failed => ("Failed", p.danger),
            JobStatus::Stopped => ("Stopped", p.muted),
        };
        widgets::status_pill(
            ui.painter(),
            pos2(mx + meta_w + 10.0, row.center().y),
            text,
            fg,
            theme::tint(fg, if p.dark { 0.16 } else { 0.10 }),
        );

        let ar = Rect::from_min_max(
            pos2(row.max.x - act_w, row.min.y + 7.0),
            pos2(row.max.x, row.max.y - 7.0),
        );
        // a child Ui that doesn't move the list's cursor (rows stay 46 px apart)
        let mut actions = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(ar)
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
        );
        {
            let ui = &mut actions;
            match j.status {
                JobStatus::Running | JobStatus::Waiting => {
                    if widgets::icon_button(ui, p, Icon::Close, "Stop", true).clicked() {
                        self.uploads.stop(j.id);
                    }
                }
                JobStatus::Failed | JobStatus::Stopped => {
                    if widgets::secondary(ui, p, "Retry", None).clicked() {
                        self.uploads.retry(j.id);
                    }
                }
                JobStatus::Done => {
                    if let Some(link) = &j.link
                        && widgets::secondary(ui, p, "Copy link", Some(Icon::Link))
                            .on_hover_text(link)
                            .clicked()
                    {
                        ui.ctx().copy_text(link.clone());
                        self.toast("Link copied");
                    }
                }
            }
        }
    }
}
