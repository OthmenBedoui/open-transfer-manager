//! New download, settings and delete dialogs (modal windows).

use super::icons::{self, Icon};
use super::theme::{self, Palette};
use super::widgets;
use crate::manager::{Manager, lock};
use crate::media::{self, Choice, MediaInfo};
use crate::model::*;
use crate::{browsers, http, paths};
use egui::{Color32, RichText, Stroke, vec2};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn modal_frame(p: &Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(p.surf)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(14)
        .inner_margin(egui::Margin::same(22))
        .shadow(egui::epaint::Shadow {
            offset: [0, 12],
            blur: 40,
            spread: 0,
            color: Color32::from_black_alpha(90),
        })
}

fn label_small(ui: &mut egui::Ui, p: &Palette, t: &str) {
    ui.label(super::text::job(t, theme::font(12.0), p.muted));
}

fn pick_folder(current: &std::path::Path) -> Option<PathBuf> {
    rfd::FileDialog::new().set_directory(current).pick_folder()
}

// ===================================================================== new download

#[derive(Clone)]
enum Probe {
    Idle,
    Loading,
    File {
        size: Option<u64>,
        resumable: bool,
        name: String,
    },
    Media(MediaInfo),
    Failed(String),
}

#[derive(PartialEq, Clone, Copy)]
enum Mode {
    File,
    Media,
}

pub struct AddDialog {
    url: String,
    last_probed: String,
    mode: Mode,
    file_name: String,
    folder: String,
    folder_touched: bool,
    category: String,
    opts: MediaOpts,
    focus_url: bool,
    confirmation: bool,
    /// Site that sent the browser request, and whether to trust it from now on.
    trust_site: String,
    trust: bool,

    headers: Vec<(String, String)>,
    probe: Arc<Mutex<(u64, Probe)>>,
    probe_id: u64,
    edited_at: std::time::Instant,
}

impl AddDialog {
    pub fn new(mgr: &Manager, url: &str) -> Self {
        let url = url.trim().to_string();
        let mut d = AddDialog {
            url: url.clone(),
            last_probed: String::new(),
            mode: Mode::File,
            file_name: String::new(),
            folder: String::new(),
            folder_touched: false,
            category: GENERAL.into(),
            opts: mgr.settings().media,
            focus_url: true,
            confirmation: false,
            trust_site: String::new(),
            trust: false,

            headers: vec![],
            probe: Arc::new(Mutex::new((0, Probe::Idle))),
            probe_id: 0,
            edited_at: std::time::Instant::now() - std::time::Duration::from_secs(5),
        };
        d.url_changed(mgr);
        d
    }

    pub fn probing(&self) -> bool {
        matches!(lock(&self.probe).1, Probe::Loading)
            || (self.last_probed.is_empty() && !self.url.is_empty())
    }

    pub fn from_request(mgr: &Manager, request: DownloadRequest) -> Self {
        let mut dialog = Self::new(mgr, &request.url);
        dialog.confirmation = true;
        dialog.trust_site = request.site();
        dialog.headers = request.headers;
        if let Some(media) = request.media {
            dialog.mode = if media { Mode::Media } else { Mode::File };
        }
        if let Some(opts) = request.opts {
            dialog.opts = opts;
        }
        if !request.filename.is_empty() {
            dialog.file_name = safe_name(&request.filename);
            dialog.category = mgr.category_for(&dialog.file_name);
        }
        dialog.update_folder(mgr);
        dialog
    }

    /// Native UI screenshot fixture; never probes the network or starts a file.
    pub fn preview_confirmation(mgr: &Manager) -> Self {
        let mut dialog = Self::from_request(
            mgr,
            DownloadRequest {
                url: "https://example.org/report.pdf".into(),
                filename: "تقرير العمل 2026.pdf".into(),
                headers: vec![],
                media: Some(false),
                opts: None,
            },
        );
        dialog.last_probed = dialog.url.clone();
        *lock(&dialog.probe) = (
            dialog.probe_id,
            Probe::File {
                size: Some(2841002),
                resumable: true,
                name: dialog.file_name.clone(),
            },
        );
        dialog
    }

    /// Used by screenshot mode to show the MP3 choices.
    pub fn prefer_audio(&mut self) {
        self.mode = Mode::Media;
        self.opts.quality = "audio".into();
        self.opts.audio_format = "mp3".into();
    }

    fn url_changed(&mut self, mgr: &Manager) {
        // Credentials supplied by a browser belong only to the original URL.
        self.headers.clear();
        // Invalidate the previous probe immediately, not after the debounce delay.
        // A slow answer for the old URL must not rename the new download.
        self.probe_id += 1;
        *lock(&self.probe) = (self.probe_id, Probe::Idle);
        self.last_probed.clear();
        self.url = self.url.trim().to_string();
        let url = self.url.trim().to_string();
        self.mode = if media::looks_like_media_page(&url) {
            Mode::Media
        } else {
            Mode::File
        };
        if self.mode == Mode::File {
            self.file_name = name_from_url(&url);
            self.category = mgr.category_for(&self.file_name);
        }
        self.update_folder(mgr);
        self.edited_at = std::time::Instant::now();
    }

    fn update_folder(&mut self, mgr: &Manager) {
        if self.folder_touched {
            return;
        }
        let cat = match self.mode {
            Mode::File => self.category.clone(),
            Mode::Media => {
                if self.opts.quality == "audio" {
                    "Music".into()
                } else {
                    "Video".into()
                }
            }
        };
        self.folder = mgr.folder_for(&cat).display().to_string();
    }

    fn start_probe(&mut self, mgr: &Manager) {
        let url = self.url.trim().to_string();
        if !(url.starts_with("http://") || url.starts_with("https://")) || url == self.last_probed {
            return;
        }
        self.last_probed = url.clone();
        self.probe_id += 1;
        let id = self.probe_id;
        *lock(&self.probe) = (id, Probe::Loading);
        let slot = self.probe.clone();
        let mgr = mgr.clone();
        let mode = self.mode;
        let headers = self.headers.clone();
        std::thread::spawn(move || {
            let r = if mode == Mode::Media {
                media::probe(&mgr, &url, &headers).map(Probe::Media)
            } else {
                http::probe(&mgr, &url, &headers).map(|(size, resumable, name)| Probe::File {
                    size,
                    resumable,
                    name,
                })
            };
            let mut s = lock(&slot);
            if s.0 == id {
                s.1 = r.unwrap_or_else(Probe::Failed);
            }
            drop(s);
            mgr.repaint();
        });
    }

    /// Returns false when the dialog should close.
    pub fn show(&mut self, ctx: &egui::Context, mgr: &Manager, p: &Palette) -> bool {
        // probe after typing pauses
        if self.edited_at.elapsed() > std::time::Duration::from_millis(600) {
            self.start_probe(mgr);
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(650));
        }
        let probe = lock(&self.probe).1.clone();
        if let Probe::File { name, .. } = &probe
            && !name.is_empty()
            && self.file_name == name_from_url(self.url.trim())
            && &self.file_name != name
        {
            self.file_name = name.clone();
            self.category = mgr.category_for(name);
            self.update_folder(mgr);
        }
        let mut keep = true;
        let mut submit: Option<bool> = None;
        let resp = egui::Modal::new(egui::Id::unique("add-dialog"))
            .frame(modal_frame(p))
            .show(ctx, |ui| {
                ui.set_width(560.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(if self.confirmation {
                            "Confirmer le téléchargement"
                        } else {
                            "New download"
                        })
                        .font(theme::bold(18.0)),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::icon_button(ui, p, Icon::Close, "Close (Esc)", true).clicked() {
                            keep = false;
                        }
                    });
                });
                ui.add_space(6.0);
                if self.confirmation {
                    label_small(
                        ui,
                        p,
                        "Vérifiez le fichier et son dossier avant de confirmer.",
                    );
                }
                label_small(ui, p, "Link");
                let r = widgets::text_input(
                    ui,
                    p,
                    &mut self.url,
                    "https://…",
                    ui.available_width() - 22.0,
                );
                if self.focus_url {
                    r.request_focus();
                    let mut state =
                        egui::text_edit::TextEditState::load(ctx, r.id).unwrap_or_default();
                    state
                        .cursor
                        .set_char_range(Some(egui::text::CCursorRange::two(
                            egui::text::CCursor::new(0),
                            egui::text::CCursor::new(self.url.chars().count()),
                        )));
                    state.store(ctx, r.id);
                    self.focus_url = false;
                }
                if r.changed() {
                    self.url_changed(mgr);
                }
                ui.add_space(8.0);
                let mut mode = self.mode;
                if widgets::segmented(
                    ui,
                    p,
                    &mut mode,
                    &[(Mode::File, "File"), (Mode::Media, "Video / audio")],
                    275.0,
                ) {
                    self.probe_id += 1;
                    *lock(&self.probe) = (self.probe_id, Probe::Idle);
                    self.mode = mode;
                    self.last_probed.clear();
                    self.update_folder(mgr);
                    self.edited_at = std::time::Instant::now() - std::time::Duration::from_secs(5);
                }
                ui.add_space(10.0);
                let before_opts = self.opts.clone();
                match self.mode {
                    Mode::File => self.file_section(ui, mgr, p, &probe),
                    Mode::Media => self.media_section(ui, mgr, p, &probe),
                }
                if before_opts.quality != self.opts.quality
                    || before_opts.audio_format != self.opts.audio_format
                    || before_opts.audio_kbps != self.opts.audio_kbps
                {
                    self.opts.format_id = None;
                }
                ui.add_space(10.0);
                label_small(ui, p, "Save to");
                ui.horizontal(|ui| {
                    if widgets::text_input(
                        ui,
                        p,
                        &mut self.folder,
                        "",
                        ui.available_width() - 110.0,
                    )
                    .changed()
                    {
                        self.folder_touched = true;
                    }
                    if widgets::secondary(ui, p, "Browse…", Some(Icon::Folder)).clicked()
                        && let Some(f) = pick_folder(std::path::Path::new(&self.folder))
                    {
                        self.folder = f.display().to_string();
                        self.folder_touched = true;
                    }
                });
                if self.confirmation && !self.trust_site.is_empty() {
                    ui.add_space(8.0);
                    ui.checkbox(
                        &mut self.trust,
                        format!(
                            "Toujours autoriser les téléchargements depuis {}",
                            self.trust_site
                        ),
                    );
                    label_small(
                        ui,
                        p,
                        "Ils démarreront sans cette fenêtre. Modifiable dans Réglages → Browsers.",
                    );
                }
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    if widgets::ghost(
                        ui,
                        p,
                        if self.confirmation {
                            "Annuler"
                        } else {
                            "Cancel"
                        },
                    )
                    .clicked()
                    {
                        keep = false;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::primary(
                            ui,
                            p,
                            if self.confirmation {
                                "Télécharger"
                            } else {
                                "Download"
                            },
                            Some(Icon::Download),
                        )
                        .clicked()
                        {
                            submit = Some(true);
                        }
                        if widgets::secondary(
                            ui,
                            p,
                            if self.confirmation {
                                "Ajouter à la file"
                            } else {
                                "Add to queue"
                            },
                            Some(Icon::Queue),
                        )
                        .clicked()
                        {
                            submit = Some(false);
                        }
                    });
                });
                if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    submit = Some(true);
                }
            });
        if resp.should_close() {
            keep = false;
        }
        if let Some(start) = submit {
            let url = self.url.trim().to_string();
            if self.confirmation && self.trust {
                let site = self.trust_site.clone();
                mgr.update_settings(|s| s.trust_site(&site));
            }
            if url.starts_with("http://") || url.starts_with("https://") {
                let folder = PathBuf::from(self.folder.trim());
                match self.mode {
                    Mode::Media => {
                        let title = if let Probe::Media(i) = &probe {
                            i.title.clone()
                        } else {
                            String::new()
                        };
                        let mut o = self.opts.clone();
                        if !self.confirmation {
                            o.format_id = None;
                        }
                        mgr.add_media(
                            &url,
                            o,
                            Some(folder),
                            &title,
                            start,
                            !start,
                            self.headers.clone(),
                        );
                    }
                    Mode::File => {
                        let name = if self.file_name.trim().is_empty() {
                            name_from_url(&url)
                        } else {
                            self.file_name.trim().to_string()
                        };
                        let r = mgr.add_http(
                            &url,
                            &name,
                            &folder,
                            &self.category,
                            start,
                            !start,
                            self.headers.clone(),
                        );
                        if let Probe::File { size, .. } = probe {
                            lock(&r).size = size;
                        }
                    }
                }
                keep = false;
            }
        }
        keep
    }

    fn file_section(&mut self, ui: &mut egui::Ui, mgr: &Manager, p: &Palette, probe: &Probe) {
        label_small(ui, p, "File name");
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(38.0, 38.0), egui::Sense::hover());
            widgets::type_badge(ui.painter(), r, &self.file_name, p.dark);
            widgets::text_input(
                ui,
                p,
                &mut self.file_name,
                "file name",
                ui.available_width() - 22.0,
            );
        });
        if super::text::contains_arabic(&self.file_name) {
            label_small(ui, p, "Aperçu du nom :");
            ui.label(super::text::job(&self.file_name, theme::font(14.0), p.text));
        }
        ui.horizontal(|ui| {
            label_small(ui, p, "Category");
            let cats: Vec<String> = mgr
                .settings()
                .categories
                .iter()
                .map(|c| c.name.clone())
                .chain([GENERAL.to_string()])
                .collect();
            let before = self.category.clone();
            egui::ComboBox::from_id_salt("cat")
                .selected_text(&self.category)
                .width(160.0)
                .show_ui(ui, |ui| {
                    for c in cats {
                        ui.selectable_value(&mut self.category, c.clone(), c);
                    }
                });
            if before != self.category {
                self.update_folder(mgr);
            }
        });
        let info = match probe {
            Probe::Loading => ("Getting information…".to_string(), p.muted),
            Probe::File {
                size, resumable, ..
            } => (
                format!(
                    "{} · {}",
                    size.map(|s| fmt_size(Some(s)))
                        .unwrap_or("unknown size".into()),
                    if *resumable {
                        "can pause and resume, multi-connection"
                    } else {
                        "single connection, cannot resume"
                    }
                ),
                p.muted,
            ),
            Probe::Failed(e) => (format!("Could not get information: {e}"), p.danger),
            _ => (String::new(), p.muted),
        };
        if !info.0.is_empty() {
            ui.label(RichText::new(info.0).size(12.0).color(info.1));
        }
        if media::looks_like_media_page(self.url.trim()) {
            ui.label(
                RichText::new(
                    "This link is a video page: choose “Video / audio” to get the actual video.",
                )
                .size(12.0)
                .color(p.warn),
            );
        }
    }

    fn media_section(&mut self, ui: &mut egui::Ui, mgr: &Manager, p: &Palette, probe: &Probe) {
        // video card
        let frame = egui::Frame::new()
            .fill(if p.dark {
                Color32::from_rgb(0x0F, 0x11, 0x17)
            } else {
                p.alt
            })
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(10)
            .inner_margin(egui::Margin::same(12));
        let mut duration = None;
        frame.show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(96.0, 54.0), egui::Sense::hover());
                ui.painter().rect_filled(r, 7, p.border);
                icons::paint(
                    ui.painter(),
                    egui::Rect::from_center_size(r.center(), vec2(22.0, 22.0)),
                    Icon::Play,
                    p.muted,
                );
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 3.0;
                    match probe {
                        Probe::Media(i) => {
                            duration = i.duration;
                            ui.add(
                                egui::Label::new(super::text::job(
                                    &i.title,
                                    theme::bold(14.0),
                                    p.text,
                                ))
                                .truncate(),
                            );
                            let mut parts = vec![];
                            if !i.uploader.is_empty() {
                                parts.push(i.uploader.clone());
                            }
                            if let Some(d) = i.duration {
                                parts.push(fmt_duration(d));
                            }
                            if !i.extractor.is_empty() {
                                parts.push(i.extractor.clone());
                            }
                            if i.is_playlist {
                                parts.push(format!("playlist of {}", i.count));
                            }
                            label_small(ui, p, &parts.join(" · "));
                        }
                        Probe::Loading => {
                            ui.spinner();
                            label_small(ui, p, "Asking the site for available formats…");
                        }
                        Probe::Failed(e) => {
                            ui.label(RichText::new(e).size(12.0).color(p.danger));
                        }
                        _ => label_small(
                            ui,
                            p,
                            "Paste a link to a video page (YouTube, TikTok, Instagram, X…)",
                        ),
                    }
                });
            });
        });
        ui.add_space(8.0);
        label_small(ui, p, "Quality");
        let choices: Vec<Choice> = if let Probe::Media(i) = probe {
            i.choices.clone()
        } else {
            Vec::new()
        };
        ui.horizontal_wrapped(|ui| {
            let mut items: Vec<(String, String)> = vec![("best".into(), "Best".into())];
            let heights: Vec<String> = if choices.is_empty() {
                vec!["2160", "1080", "720", "480", "360"]
                    .into_iter()
                    .map(String::from)
                    .collect()
            } else {
                choices
                    .iter()
                    .filter(|c| c.group == "video")
                    .map(|c| c.opts.quality.clone())
                    .collect()
            };
            for h in heights {
                let label = match h.as_str() {
                    "2160" => "4K".to_string(),
                    "4320" => "8K".into(),
                    "1440" => "2K".into(),
                    _ => format!("{h}p"),
                };
                items.push((h, label));
            }
            items.push(("audio".into(), "Audio only".into()));
            for (q, label) in items {
                if widgets::chip(ui, p, &label, self.opts.quality == q).clicked() {
                    self.opts.quality = q;
                    self.update_folder(mgr);
                }
            }
        });
        if self.opts.quality == "audio" {
            ui.add_space(8.0);
            label_small(
                ui,
                p,
                "Convert to MP3 — choose sound quality and file size (Standard is recommended)",
            );
            let have_ffmpeg = media::ffmpeg_path().is_some();
            ui.horizontal(|ui| {
                for (kbps, label, note) in [
                    (320u32, "High", "320 kbps"),
                    (192, "Standard", "192 kbps"),
                    (128, "Small", "128 kbps"),
                ] {
                    let on = self.opts.audio_format == "mp3" && self.opts.audio_kbps == kbps;
                    let size = duration
                        .map(|d| fmt_size(Some((kbps as f64 * 1000.0 / 8.0 * d) as u64)))
                        .map(|s| format!("≈ {s}"))
                        .unwrap_or_default();
                    if audio_card(ui, p, label, note, &size, on, have_ffmpeg).clicked()
                        && have_ffmpeg
                    {
                        self.opts.audio_format = "mp3".into();
                        self.opts.audio_kbps = kbps;
                    }
                }
                let orig = choices.iter().find(|c| c.id == "audio-original");
                let on = self.opts.audio_format == "original" || !have_ffmpeg;
                let size = orig
                    .and_then(|c| c.size)
                    .map(|s| format!("≈ {}", fmt_size(Some(s))))
                    .unwrap_or_default();
                if audio_card(ui, p, "Original", "no conversion", &size, on, true).clicked() {
                    self.opts.audio_format = "original".into();
                }
            });
            if !have_ffmpeg {
                ui.label(RichText::new("ffmpeg is not installed: MP3 conversion needs it. The original audio (M4A) is used instead.").size(12.0).color(p.warn));
            }
        } else if media::ffmpeg_path().is_none() {
            ui.label(
                RichText::new(
                    "ffmpeg is not installed: higher qualities need it to join video and audio.",
                )
                .size(12.0)
                .color(p.warn),
            );
        }
        if let Probe::Media(i) = probe
            && i.is_playlist
        {
            ui.checkbox(
                &mut self.opts.playlist,
                format!("Download the whole playlist ({} items)", i.count),
            );
        }
    }
}

fn audio_card(
    ui: &mut egui::Ui,
    p: &Palette,
    label: &str,
    note: &str,
    size: &str,
    on: bool,
    enabled: bool,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(
        vec2(128.0, 70.0),
        if enabled {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );
    let (bg, border) = if on {
        (theme::tint(p.btn, if p.dark { 0.2 } else { 0.1 }), p.btn)
    } else if resp.hovered() && enabled {
        (p.hover, p.border)
    } else {
        (Color32::TRANSPARENT, p.border)
    };
    ui.painter().rect_filled(rect, 10, bg);
    ui.painter()
        .rect_stroke(rect, 10, Stroke::new(1.0, border), egui::StrokeKind::Inside);
    let fg = if enabled { p.text } else { p.muted };
    ui.painter().text(
        rect.min + vec2(12.0, 12.0),
        egui::Align2::LEFT_TOP,
        label,
        theme::bold(13.0),
        fg,
    );
    ui.painter().text(
        rect.min + vec2(12.0, 32.0),
        egui::Align2::LEFT_TOP,
        note,
        theme::font(11.5),
        p.muted,
    );
    ui.painter().text(
        rect.min + vec2(12.0, 50.0),
        egui::Align2::LEFT_TOP,
        size,
        theme::font(11.5),
        fg,
    );
    resp.on_hover_cursor(if enabled {
        egui::CursorIcon::PointingHand
    } else {
        egui::CursorIcon::NotAllowed
    })
}

// ===================================================================== delete

pub struct DeleteDialog {
    ids: Vec<u64>,
    delete_files: bool,
}

impl DeleteDialog {
    pub fn new(ids: Vec<u64>) -> Self {
        Self {
            ids,
            delete_files: false,
        }
    }

    /// Some(true) = removed, Some(false) = cancelled, None = still open.
    pub fn show(&mut self, ctx: &egui::Context, mgr: &Manager, p: &Palette) -> Option<bool> {
        let mut out = None;
        let names: Vec<String> = self
            .ids
            .iter()
            .filter_map(|id| mgr.find(*id))
            .map(|d| lock(&d).display_name().to_string())
            .collect();
        let any_complete = self
            .ids
            .iter()
            .filter_map(|id| mgr.find(*id))
            .any(|d| lock(&d).status == Status::Complete);
        let resp = egui::Modal::new(egui::Id::unique("delete-dialog"))
            .frame(modal_frame(p))
            .show(ctx, |ui| {
                ui.set_width(420.0);
                let title = if names.len() == 1 {
                    format!("Remove “{}”?", names[0])
                } else {
                    format!("Remove {} downloads?", names.len())
                };
                ui.add(egui::Label::new(RichText::new(title).font(theme::bold(15.0))).wrap());
                ui.add_space(6.0);
                if any_complete {
                    ui.checkbox(
                        &mut self.delete_files,
                        "Also delete the downloaded files from disk",
                    );
                }
                label_small(ui, p, "Unfinished parts are always cleaned up.");
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::primary(ui, p, "Remove", Some(Icon::Trash)).clicked() {
                            for id in &self.ids {
                                mgr.remove(*id, self.delete_files);
                            }
                            out = Some(true);
                        }
                        if widgets::secondary(ui, p, "Cancel", None).clicked() {
                            out = Some(false);
                        }
                    });
                });
            });
        if resp.should_close() && out.is_none() {
            out = Some(false);
        }
        out
    }
}

// ===================================================================== settings

const TABS: &[&str] = &[
    "General",
    "Connections",
    "Speed & queue",
    "Categories",
    "Video & audio",
    "Browsers",
    "About",
];

pub struct SettingsDialog {
    tab: usize,
    s: Settings,
    servers: String,
    cat_idx: usize,
    yt_status: Arc<Mutex<Option<String>>>,
    yt_busy: Arc<Mutex<bool>>,
    browser_msg: String,
    limit_text: String,
}

impl SettingsDialog {
    pub fn new(mgr: &Manager, tab: usize) -> Self {
        let s = mgr.settings();
        let servers = s
            .server_connections
            .iter()
            .map(|(h, n)| format!("{h} = {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        let limit_text = s.limit_kb.to_string();
        SettingsDialog {
            tab,
            s,
            servers,
            cat_idx: 0,
            yt_status: Arc::new(Mutex::new(None)),
            yt_busy: Arc::new(Mutex::new(false)),
            browser_msg: String::new(),
            limit_text,
        }
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        mgr: &Manager,
        p: &Palette,
        logo: Option<&egui::TextureHandle>,
    ) -> bool {
        let mut keep = true;
        let resp = egui::Modal::new(egui::Id::unique("settings-dialog"))
            .frame(modal_frame(p).inner_margin(egui::Margin::same(0)))
            .show(ctx, |ui| {
                ui.set_width(820.0);
                ui.set_height(580.0);
                ui.horizontal_top(|ui| {
                    // tabs
                    egui::Frame::new()
                        .fill(p.bg)
                        .inner_margin(egui::Margin::same(12))
                        .corner_radius(egui::CornerRadius {
                            nw: 14,
                            sw: 14,
                            ne: 0,
                            se: 0,
                        })
                        .show(ui, |ui| {
                            ui.set_width(176.0);
                            ui.set_height(556.0);
                            ui.vertical(|ui| {
                                ui.label(RichText::new("Settings").font(theme::bold(18.0)));
                                ui.add_space(10.0);
                                for (i, t) in TABS.iter().enumerate() {
                                    let (r, resp) = ui.allocate_exact_size(
                                        vec2(ui.available_width(), 34.0),
                                        egui::Sense::click(),
                                    );
                                    let on = self.tab == i;
                                    if on {
                                        ui.painter().rect_filled(r, 8, p.soft);
                                    } else if resp.hovered() {
                                        ui.painter().rect_filled(r, 8, p.hover);
                                    }
                                    ui.painter().text(
                                        r.left_center() + vec2(12.0, 0.0),
                                        egui::Align2::LEFT_CENTER,
                                        *t,
                                        if on {
                                            theme::bold(13.5)
                                        } else {
                                            theme::font(13.5)
                                        },
                                        if on {
                                            if p.dark { Color32::WHITE } else { p.accent }
                                        } else {
                                            p.text
                                        },
                                    );
                                    if resp.clicked() {
                                        self.tab = i;
                                    }
                                }
                            });
                        });
                    // content
                    ui.vertical(|ui| {
                        ui.add_space(22.0);
                        ui.set_width(600.0);
                        ui.set_max_width(600.0);
                        egui::ScrollArea::vertical()
                            .max_height(470.0)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.set_width(580.0);
                                match self.tab {
                                    0 => self.general(ui, p),
                                    1 => self.connections(ui, p),
                                    2 => self.speed(ui, p),
                                    3 => self.categories(ui, p),
                                    4 => self.media(ui, p, mgr),
                                    5 => self.browsers(ui, p),
                                    _ => about(ui, p, logo),
                                }
                            });
                        ui.add_space(12.0);
                        ui.horizontal(|ui| {
                            ui.add_space(340.0);
                            if widgets::secondary(ui, p, "Cancel", None).clicked() {
                                keep = false;
                            }
                            if widgets::primary(ui, p, "Save", Some(Icon::Check)).clicked() {
                                self.save(mgr);
                                keep = false;
                            }
                        });
                    });
                });
            });
        if resp.should_close() {
            keep = false;
        }
        keep
    }

    fn save(&mut self, mgr: &Manager) {
        let mut servers = Vec::new();
        for line in self.servers.lines() {
            if let Some((h, n)) = line.split_once('=')
                && let Ok(n) = n.trim().parse::<usize>()
                && !h.trim().is_empty()
            {
                servers.push((h.trim().to_string(), n.max(1)));
            }
        }
        self.s.server_connections = servers;
        if let Ok(k) = self.limit_text.trim().parse::<u64>() {
            self.s.limit_kb = k.max(16);
        }
        let s = self.s.clone();
        mgr.update_settings(|cur| *cur = s);
    }

    fn general(&mut self, ui: &mut egui::Ui, p: &Palette) {
        heading(ui, "Appearance");
        ui.horizontal(|ui| {
            for (m, label) in [
                (ThemeMode::Light, "Light"),
                (ThemeMode::Dark, "Dark"),
                (ThemeMode::System, "Follow system"),
            ] {
                if theme_card(ui, p, m, label, self.s.theme == m).clicked() {
                    self.s.theme = m;
                }
            }
        });
        ui.add_space(14.0);
        heading(ui, "Downloads");
        label_small(
            ui,
            p,
            "Download folder (each category saves into its own subfolder)",
        );
        ui.horizontal(|ui| {
            let mut f = self.s.base_folder.display().to_string();
            if widgets::text_input(ui, p, &mut f, "", 420.0).changed() {
                self.set_base(PathBuf::from(f));
            }
            if widgets::secondary(ui, p, "Browse…", Some(Icon::Folder)).clicked()
                && let Some(d) = pick_folder(&self.s.base_folder)
            {
                self.set_base(d);
            }
        });
        ui.add_space(8.0);
        widgets::switch_row(
            ui,
            p,
            "Notify when a download finishes",
            "Desktop notification.",
            &mut self.s.notify_on_complete,
        );
        ui.separator();
        widgets::switch_row(
            ui,
            p,
            "Start minimized from the browser",
            "When the extension starts the app, keep it in the background.",
            &mut self.s.start_minimized,
        );
        ui.separator();
        widgets::switch_row(
            ui,
            p,
            "Show the details panel",
            "Progress per connection, speed and file location.",
            &mut self.s.show_details,
        );
    }

    fn set_base(&mut self, base: PathBuf) {
        let old = self.s.base_folder.clone();
        for c in self.s.categories.iter_mut() {
            if c.folder == old.join(&c.name) {
                c.folder = base.join(&c.name);
            }
        }
        self.s.base_folder = base;
    }

    fn connections(&mut self, ui: &mut egui::Ui, p: &Palette) {
        heading(ui, "Connections per download");
        widgets::segmented(
            ui,
            p,
            &mut self.s.max_connections,
            &[
                (1, "1"),
                (2, "2"),
                (4, "4"),
                (8, "8"),
                (16, "16"),
                (32, "32"),
            ],
            60.0,
        );
        label_small(
            ui,
            p,
            "More connections are faster on most servers. Some servers limit them.",
        );
        ui.add_space(12.0);
        heading(ui, "Per-server limits");
        label_small(ui, p, "One per line, for example:  example.com = 2");
        ui.add(
            egui::TextEdit::multiline(&mut self.servers)
                .desired_rows(5)
                .desired_width(560.0),
        );
    }

    fn speed(&mut self, ui: &mut egui::Ui, p: &Palette) {
        heading(ui, "Speed limit");
        widgets::switch_row(
            ui,
            p,
            "Limit total download speed",
            "",
            &mut self.s.limiter_on,
        );
        ui.horizontal(|ui| {
            label_small(ui, p, "Maximum (KB/s)");
            widgets::text_input(ui, p, &mut self.limit_text, "1024", 100.0);
        });
        widgets::switch_row(
            ui,
            p,
            "Turn the limit on at startup",
            "",
            &mut self.s.limiter_on_startup,
        );
        ui.add_space(12.0);
        heading(ui, "Queue");
        ui.horizontal(|ui| {
            label_small(ui, p, "Files at the same time");
            widgets::segmented(
                ui,
                p,
                &mut self.s.queue_at_once,
                &[(1, "1"), (2, "2"), (3, "3"), (5, "5"), (10, "10")],
                48.0,
            );
        });
        ui.horizontal(|ui| {
            label_small(ui, p, "Retries per file");
            ui.add(egui::DragValue::new(&mut self.s.retries).range(0..=100));
        });
        ui.add_space(12.0);
        heading(ui, "Scheduler (HH:MM, 24-hour clock)");
        ui.horizontal(|ui| {
            ui.checkbox(
                &mut self.s.schedule.start_on,
                "Start the queue every day at",
            );
            widgets::text_input(ui, p, &mut self.s.schedule.start_at, "02:00", 60.0);
        });
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.s.schedule.stop_on, "Stop the queue every day at");
            widgets::text_input(ui, p, &mut self.s.schedule.stop_at, "07:00", 60.0);
        });
    }

    fn categories(&mut self, ui: &mut egui::Ui, p: &Palette) {
        heading(ui, "Categories");
        let names: Vec<String> = self.s.categories.iter().map(|c| c.name.clone()).collect();
        if names.is_empty() {
            return;
        }
        self.cat_idx = self.cat_idx.min(names.len() - 1);
        egui::ComboBox::from_id_salt("catpick")
            .selected_text(&names[self.cat_idx])
            .width(200.0)
            .show_ui(ui, |ui| {
                for (i, n) in names.iter().enumerate() {
                    ui.selectable_value(&mut self.cat_idx, i, n);
                }
            });
        let c = &mut self.s.categories[self.cat_idx];
        ui.add_space(6.0);
        label_small(
            ui,
            p,
            "File types (extensions separated by spaces, * is a wildcard)",
        );
        widgets::text_input(ui, p, &mut c.exts, "", 540.0);
        label_small(ui, p, "Save to");
        ui.horizontal(|ui| {
            let mut f = c.folder.display().to_string();
            if widgets::text_input(ui, p, &mut f, "", 420.0).changed() {
                c.folder = PathBuf::from(f);
            }
            if widgets::secondary(ui, p, "Browse…", Some(Icon::Folder)).clicked()
                && let Some(d) = pick_folder(&c.folder)
            {
                c.folder = d;
            }
        });
    }

    fn media(&mut self, ui: &mut egui::Ui, p: &Palette, _mgr: &Manager) {
        heading(ui, "Video engine (yt-dlp)");
        let status = lock(&self.yt_status).clone();
        let ver = media::ytdlp_version();
        ui.horizontal(|ui| {
            ui.label(match &ver {
                Some(v) => format!("yt-dlp {v}"),
                None => "yt-dlp is not installed".into(),
            });
            let busy = *lock(&self.yt_busy);
            let label = if busy {
                "Working…"
            } else if ver.is_some() {
                "Update now"
            } else {
                "Install"
            };
            if widgets::secondary(ui, p, label, Some(Icon::Download)).clicked() && !busy {
                *lock(&self.yt_busy) = true;
                let st = self.yt_status.clone();
                let b = self.yt_busy.clone();
                let ctx = ui.ctx().clone();
                std::thread::spawn(move || {
                    let r = media::install_or_update();
                    *lock(&st) = Some(match r {
                        Ok(t) => t,
                        Err(e) => format!("Failed: {e}"),
                    });
                    *lock(&b) = false;
                    ctx.request_repaint();
                });
            }
        });
        if let Some(s) = status {
            label_small(ui, p, s.lines().last().unwrap_or(""));
        }
        label_small(
            ui,
            p,
            "Sites change often: update yt-dlp when a video stops working.",
        );
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label("ffmpeg");
            match media::ffmpeg_path() {
                Some(f) => {
                    ui.add(
                        egui::Label::new(
                            RichText::new(f.display().to_string())
                                .size(12.0)
                                .color(p.muted),
                        )
                        .truncate(),
                    );
                }
                None => {
                    ui.label(
                        RichText::new("not found — needed for MP3 and best quality").color(p.warn),
                    );
                }
            }
        });
        ui.add_space(12.0);
        heading(ui, "Defaults");
        ui.horizontal(|ui| {
            label_small(ui, p, "Video quality");
            widgets::segmented(
                ui,
                p,
                &mut self.s.media.quality,
                &[
                    ("best".to_string(), "Best"),
                    ("1080".to_string(), "1080p"),
                    ("720".to_string(), "720p"),
                    ("audio".to_string(), "Audio"),
                ],
                70.0,
            );
        });
        ui.horizontal(|ui| {
            label_small(ui, p, "MP3 quality");
            widgets::segmented(
                ui,
                p,
                &mut self.s.media.audio_kbps,
                &[(320, "320 kbps"), (192, "192 kbps"), (128, "128 kbps")],
                86.0,
            );
        });
        ui.horizontal(|ui| {
            label_small(ui, p, "Audio format");
            widgets::segmented(
                ui,
                p,
                &mut self.s.media.audio_format,
                &[
                    ("mp3".to_string(), "MP3"),
                    ("m4a".to_string(), "M4A"),
                    ("opus".to_string(), "OPUS"),
                    ("flac".to_string(), "FLAC"),
                    ("original".to_string(), "Original"),
                ],
                70.0,
            );
        });
        ui.horizontal(|ui| {
            label_small(ui, p, "Use cookies from");
            let label = if self.s.cookies_browser.is_empty() {
                "none".to_string()
            } else {
                self.s.cookies_browser.clone()
            };
            egui::ComboBox::from_id_salt("cookies")
                .selected_text(label)
                .width(140.0)
                .show_ui(ui, |ui| {
                    for b in [
                        "", "firefox", "chrome", "chromium", "brave", "edge", "opera", "vivaldi",
                        "safari",
                    ] {
                        ui.selectable_value(
                            &mut self.s.cookies_browser,
                            b.to_string(),
                            if b.is_empty() { "none" } else { b },
                        );
                    }
                });
        });
        label_small(
            ui,
            p,
            "Cookies let yt-dlp see videos you can watch while signed in (age-restricted, private).",
        );
        widgets::switch_row(
            ui,
            p,
            "Download whole playlists by default",
            "",
            &mut self.s.media.playlist,
        );
        ui.separator();
        widgets::switch_row(
            ui,
            p,
            "Add cover art and lyrics to MP3 files",
            "Sends title and artist to LRCLIB and MusicBrainz. Single songs only.",
            &mut self.s.music_tags,
        );
    }

    fn browsers(&mut self, ui: &mut egui::Ui, p: &Palette) {
        heading(ui, "Browser extension");
        label_small(
            ui,
            p,
            "The extension captures downloads, adds a Download button on videos and shows progress in the browser.",
        );
        ui.add_space(8.0);
        let st = browsers::status();
        if st.is_empty() {
            label_small(ui, p, "No supported browser found.");
        }
        egui::Grid::new("browsers")
            .num_columns(2)
            .spacing([40.0, 8.0])
            .show(ui, |ui| {
                for (name, _installed, reg) in st {
                    ui.label(name);
                    ui.label(
                        RichText::new(if reg {
                            "bridge registered"
                        } else {
                            "bridge not registered"
                        })
                        .color(if reg { p.success } else { p.muted }),
                    );
                    ui.end_row();
                }
            });
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if widgets::primary(ui, p, "Connect browsers", Some(Icon::Link)).clicked() {
                self.browser_msg = match browsers::register(false) {
                    Ok(d) => format!(
                        "Bridge registered for {} browser(s). Restart the browser if it was open.",
                        d.len()
                    ),
                    Err(e) => format!("Failed: {e}"),
                };
            }
        });
        if !self.browser_msg.is_empty() {
            label_small(ui, p, &self.browser_msg);
        }
        ui.add_space(14.0);
        heading(ui, "Sites de confiance");
        label_small(
            ui,
            p,
            "Les téléchargements envoyés par le navigateur depuis ces sites démarrent sans confirmation. \
             Tous les autres sites demandent toujours.",
        );
        if self.s.trusted_sites.is_empty() {
            label_small(
                ui,
                p,
                "Aucun site : chaque téléchargement du navigateur demande une confirmation.",
            );
        }
        let mut remove = None;
        for (i, site) in self.s.trusted_sites.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(site);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::secondary(ui, p, "Retirer", Some(Icon::Trash)).clicked() {
                        remove = Some(i);
                    }
                });
            });
        }
        if let Some(i) = remove {
            self.s.trusted_sites.remove(i);
        }
        ui.add_space(14.0);
        label_small(
            ui,
            p,
            "The bridge is registered automatically. The browser must still approve the extension.",
        );
        let chrome_folder = browsers::extension_dir().join("chromium");
        if browsers::chrome_store_id().is_some() {
            label_small(
                ui,
                p,
                "Distribution Web Store configurée : Chrome peut récupérer l’extension publiée. Confirmez son activation dans Chrome.",
            );
        } else {
            ui.label(
                RichText::new("Extension locale : installation guidée nécessaire").color(p.warn),
            );
            label_small(
                ui,
                p,
                "1. Ouvrir Chrome → Extensions et activer le mode développeur.",
            );
            label_small(
                ui,
                p,
                "2. Cliquer sur « Charger l’extension non empaquetée ».",
            );
            label_small(
                ui,
                p,
                "3. Sélectionner le dossier chromium, pas le ZIP ni manifest.json.",
            );
        }
        if widgets::primary(ui, p, "Installer / configurer Chrome", Some(Icon::Link)).clicked() {
            self.browser_msg = match browsers::open_chrome_setup() {
                Ok(()) => {
                    "Chrome ouvert. Suivez les étapes affichées pour terminer l’installation."
                        .into()
                }
                Err(e) => e,
            };
        }
        ui.horizontal(|ui| {
            if widgets::secondary(ui, p, "Copier le dossier Chrome", Some(Icon::Link)).clicked() {
                ui.ctx().copy_text(chrome_folder.display().to_string());
            }
            if widgets::secondary(ui, p, "Ouvrir le dossier Chrome", Some(Icon::Folder)).clicked() {
                paths::open_path(&chrome_folder);
            }
        });
        label_small(ui, p, &chrome_folder.display().to_string());
        if widgets::secondary(ui, p, "Autres navigateurs", Some(Icon::Folder)).clicked() {
            paths::open_path(&browsers::extension_dir());
        }
    }
}

fn heading(ui: &mut egui::Ui, t: &str) {
    ui.label(RichText::new(t).font(theme::bold(14.0)));
    ui.add_space(2.0);
}

fn theme_card(
    ui: &mut egui::Ui,
    p: &Palette,
    m: ThemeMode,
    label: &str,
    on: bool,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(178.0, 112.0), egui::Sense::click());
    let painter = ui.painter();
    painter.rect_filled(rect, 12, p.surf);
    painter.rect_stroke(
        rect,
        12,
        Stroke::new(
            if on { 2.0 } else { 1.0 },
            if on { p.btn } else { p.border },
        ),
        egui::StrokeKind::Inside,
    );
    let prev =
        egui::Rect::from_min_size(rect.min + vec2(10.0, 10.0), vec2(rect.width() - 20.0, 66.0));
    let (side, main, line) = match m {
        ThemeMode::Light => (
            theme::palette(false).side,
            Color32::WHITE,
            theme::palette(false).border,
        ),
        ThemeMode::Dark => (
            theme::palette(true).side,
            theme::palette(true).surf,
            theme::palette(true).border,
        ),
        ThemeMode::System => (
            theme::palette(false).side,
            theme::palette(true).surf,
            theme::palette(true).muted,
        ),
    };
    painter.rect_filled(prev, 7, main);
    painter.rect_filled(
        egui::Rect::from_min_size(prev.min, vec2(prev.width() * 0.3, prev.height())),
        egui::CornerRadius {
            nw: 7,
            sw: 7,
            ne: 0,
            se: 0,
        },
        side,
    );
    let x = prev.min.x + prev.width() * 0.36;
    painter.rect_filled(
        egui::Rect::from_min_size(
            egui::pos2(x, prev.min.y + 12.0),
            vec2(prev.width() * 0.4, 6.0),
        ),
        3,
        line,
    );
    widgets::progress_bar(
        painter,
        egui::Rect::from_min_size(
            egui::pos2(x, prev.min.y + 28.0),
            vec2(prev.width() * 0.55, 5.0),
        ),
        0.8,
        line,
        None,
    );
    painter.rect_filled(
        egui::Rect::from_min_size(
            egui::pos2(x, prev.min.y + 44.0),
            vec2(prev.width() * 0.3, 6.0),
        ),
        3,
        line,
    );
    painter.text(
        egui::pos2(rect.min.x + 12.0, rect.max.y - 18.0),
        egui::Align2::LEFT_CENTER,
        label,
        theme::font(13.0),
        p.text,
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn about(ui: &mut egui::Ui, p: &Palette, logo: Option<&egui::TextureHandle>) {
    ui.vertical_centered(|ui| {
        ui.add_space(20.0);
        if let Some(t) = logo {
            ui.add(egui::Image::new(t).fit_to_exact_size(vec2(84.0, 84.0)));
        }
        ui.label(RichText::new(paths::APP_NAME).font(theme::bold(22.0)));
        label_small(ui, p, &format!("Version {}", env!("CARGO_PKG_VERSION")));
        ui.add_space(10.0);
        ui.label("A light, open-source download manager for Windows, Linux and macOS.");
        ui.label("Multi-connection downloads, video and MP3 from 1000+ sites, browser integration.");
        ui.add_space(10.0);
        label_small(ui, p, "Free software under the MIT License. Video engine: yt-dlp (Unlicense). Font: Inter (OFL).");
        ui.add_space(10.0);
        if widgets::secondary(ui, p, "Copy diagnostics", Some(Icon::Link)).clicked() {
            ui.ctx().copy_text(crate::doctor());
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_confirmation_keeps_list_empty_and_preserves_request_headers() {
        let mgr = Manager::new(false);
        let mut dialog = AddDialog::from_request(
            &mgr,
            DownloadRequest {
                url: "https://example.com/a.pdf".into(),
                filename: "تقرير العمل.pdf".into(),
                headers: vec![("Cookie".into(), "test=value".into())],
                media: Some(false),
                opts: None,
            },
        );
        assert!(mgr.list().is_empty());
        assert_eq!(dialog.headers.len(), 1);
        assert_eq!(dialog.file_name, "تقرير العمل.pdf");
        dialog.last_probed = dialog.url.clone();
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        let palette = theme::palette(false);
        let mut frame = ctx.run_ui(egui::RawInput::default(), |ui| {
            dialog.show(ui.ctx(), &mgr, &palette);
        });
        frame.textures_delta.clear();
        let input = egui::RawInput {
            events: vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            ..Default::default()
        };
        let mut keep = true;
        let mut frame = ctx.run_ui(input, |ui| {
            keep = dialog.show(ui.ctx(), &mgr, &palette);
        });
        frame.textures_delta.clear();
        assert!(!keep);
        assert!(mgr.list().is_empty());
        dialog.url = "https://other.example/b.pdf".into();
        dialog.url_changed(&mgr);
        assert!(
            dialog.headers.is_empty(),
            "changing URL must discard browser credentials"
        );
    }

    #[test]
    fn new_dialog_focuses_url_and_accepts_native_paste() {
        let mgr = Manager::new(false);
        let mut dialog = AddDialog::new(&mgr, "");
        let ctx = egui::Context::default();
        let palette = theme::palette(false);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            dialog.show(ui.ctx(), &mgr, &palette);
        });
        output.textures_delta.clear();
        assert!(ctx.memory(|m| m.focused().is_some()));
        let input = egui::RawInput {
            events: vec![egui::Event::Paste("https://example.com/new.zip".into())],
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            dialog.show(ui.ctx(), &mgr, &palette);
        });
        output.textures_delta.clear();
        assert_eq!(dialog.url, "https://example.com/new.zip");
        assert_eq!(dialog.file_name, "new.zip");
        assert_eq!(dialog.category, "Archives");
    }

    #[test]
    fn changing_url_invalidates_inflight_probe_immediately() {
        let mgr = Manager::new(false);
        let mut dialog = AddDialog::new(&mgr, "https://example.com/old.zip");
        let old_id = dialog.probe_id;
        *lock(&dialog.probe) = (old_id, Probe::Loading);
        dialog.last_probed = dialog.url.clone();
        dialog.url = "https://example.com/new.pdf".into();
        dialog.url_changed(&mgr);
        assert_ne!(lock(&dialog.probe).0, old_id);
        assert!(matches!(lock(&dialog.probe).1, Probe::Idle));
        assert_eq!(dialog.file_name, "new.pdf");
        assert_eq!(dialog.category, "Documents");
    }
}
