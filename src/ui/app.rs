//! Main window: sidebar, toolbar, download list, details panel and status bar.

use super::dialogs::{AddDialog, DeleteDialog, SettingsDialog};
use super::icons::{self, Icon};
use super::theme::{self, Palette};
use super::widgets;
use crate::api::ApiServer;
use crate::manager::{DlRef, Event, Manager, lock};
use crate::model::*;
use crate::{media, paths};
use egui::{Align2, Color32, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone, PartialEq, Debug)]
pub enum View {
    All,
    Active,
    Paused,
    Done,
    Failed,
    Category(String),
}

impl View {
    fn title(&self) -> String {
        match self {
            View::All => "All downloads".into(),
            View::Active => "Active".into(),
            View::Paused => "Paused & queued".into(),
            View::Done => "Completed".into(),
            View::Failed => "Failed".into(),
            View::Category(c) => c.clone(),
        }
    }
    fn accepts(&self, d: &Download) -> bool {
        match self {
            View::All => true,
            View::Active => d.running(),
            View::Paused => !d.running() && d.status == Status::Stopped,
            View::Done => d.status == Status::Complete,
            View::Failed => d.status == Status::Error,
            View::Category(c) => &d.category == c,
        }
    }
}

pub struct OtmApp {
    pub mgr: Manager,
    api: Option<ApiServer>,
    view: View,
    search: String,
    selected: Option<u64>,
    logo: Option<egui::TextureHandle>,
    pub add: Option<AddDialog>,
    pub settings: Option<SettingsDialog>,
    pub delete: Option<DeleteDialog>,
    toast: Option<(String, Instant)>,
    last_tick: Instant,
    screenshot: Option<(String, u32)>,
    pending_urls: Vec<String>,
    pending_confirmations: std::collections::VecDeque<DownloadRequest>,
    dark: bool,
    bridge: (Instant, Vec<String>),
}

pub fn run(
    urls: Vec<String>,
    from_browser: bool,
    screenshot: Option<String>,
    setup_browsers: bool,
) -> i32 {
    let persist = screenshot.is_none();
    let mgr = Manager::new(persist);
    mgr.enable_confirmation_ui();
    let api = if persist {
        ApiServer::start(mgr.clone()).ok()
    } else {
        None
    };
    let icon = image::load_from_memory(include_bytes!("../../assets/otm-64.png"))
        .ok()
        .map(|img| {
            let img = img.to_rgba8();
            egui::IconData {
                width: img.width(),
                height: img.height(),
                rgba: img.into_raw(),
            }
        });
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(paths::APP_NAME)
        .with_app_id(paths::APP_ID)
        .with_inner_size([1180.0, 760.0])
        .with_min_inner_size([760.0, 480.0]);
    if let Some(i) = icon {
        viewport = viewport.with_icon(Arc::new(i));
    }
    let start_minimized = from_browser && mgr.settings().start_minimized;
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    let mgr2 = mgr.clone();
    let result = eframe::run_native(
        paths::APP_NAME,
        options,
        Box::new(move |cc| {
            theme::install_fonts(&cc.egui_ctx);
            let ctx = cc.egui_ctx.clone();
            mgr2.set_repaint(move || ctx.request_repaint());
            let logo = image::load_from_memory(include_bytes!("../../assets/otm-64.png"))
                .ok()
                .map(|img| {
                    let img = img.to_rgba8();
                    let ci = egui::ColorImage::from_rgba_unmultiplied(
                        [img.width() as usize, img.height() as usize],
                        img.as_raw(),
                    );
                    cc.egui_ctx
                        .load_texture("logo", ci, egui::TextureOptions::LINEAR)
                });
            let mut dialog_add = None;
            let mut dialog_settings = setup_browsers.then(|| SettingsDialog::new(&mgr2, 5));
            if screenshot.is_some() {
                *crate::ui::app::DEMO.lock().unwrap() = true;
                demo_downloads(&mgr2);
                // OTM_SHOT_THEME=light|dark, OTM_SHOT_DIALOG=add|settings|settings-media (docs / visual checks)
                match std::env::var("OTM_SHOT_THEME").as_deref() {
                    Ok("light") => mgr2.update_settings(|s| s.theme = ThemeMode::Light),
                    Ok("dark") => mgr2.update_settings(|s| s.theme = ThemeMode::Dark),
                    _ => {}
                }
                match std::env::var("OTM_SHOT_DIALOG").as_deref() {
                    Ok("add") => {
                        let mut d = AddDialog::new(
                            &mgr2,
                            &std::env::var("OTM_SHOT_URL").unwrap_or_default(),
                        );
                        d.prefer_audio();
                        dialog_add = Some(d);
                    }
                    Ok("settings") => dialog_settings = Some(SettingsDialog::new(&mgr2, 0)),
                    Ok("settings-media") => dialog_settings = Some(SettingsDialog::new(&mgr2, 4)),
                    Ok("confirm") => dialog_add = Some(AddDialog::preview_confirmation(&mgr2)),
                    _ => {}
                }
            }
            if start_minimized {
                cc.egui_ctx
                    .send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            }
            Ok(Box::new(OtmApp {
                mgr: mgr2.clone(),
                api,
                view: View::All,
                search: String::new(),
                selected: None,
                logo,
                add: dialog_add,
                settings: dialog_settings,
                delete: None,
                toast: None,
                last_tick: Instant::now() - Duration::from_secs(1),
                screenshot: screenshot.map(|s| (s, 0)),
                pending_urls: urls,
                pending_confirmations: Default::default(),
                dark: true,
                bridge: (Instant::now() - Duration::from_secs(60), Vec::new()),
            }))
        }),
    );
    mgr.shutdown();
    if result.is_err() { 1 } else { 0 }
}

pub static DEMO: std::sync::Mutex<bool> = std::sync::Mutex::new(false);

impl OtmApp {
    pub fn palette(&self) -> Palette {
        theme::palette(self.dark)
    }

    pub fn toast(&mut self, text: impl Into<String>) {
        self.toast = Some((text.into(), Instant::now()));
    }

    fn selected_dl(&self) -> Option<DlRef> {
        self.selected.and_then(|id| self.mgr.find(id))
    }

    fn visible(&self) -> Vec<DlRef> {
        let q = self.search.to_lowercase();
        let mut v: Vec<DlRef> = self
            .mgr
            .list()
            .into_iter()
            .filter(|d| {
                let d = lock(d);
                self.view.accepts(&d)
                    && (q.is_empty()
                        || d.display_name().to_lowercase().contains(&q)
                        || d.url.to_lowercase().contains(&q))
            })
            .collect();
        v.reverse(); // newest first
        v
    }

    fn handle_events(&mut self, ctx: &egui::Context) {
        let notify = self.mgr.settings().notify_on_complete;
        for e in self.mgr.take_events() {
            match e {
                Event::Finished { name, path } => {
                    self.toast(format!("Saved: {}", path.display()));
                    if notify {
                        paths::notify("Download complete", &name);
                    }
                }
                Event::Failed { name, error } => self.toast(format!("{name}: {error}")),
                Event::ShowWindow => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                Event::ShowBrowserSetup => {
                    self.settings = Some(SettingsDialog::new(&self.mgr, 5));
                }
                Event::Added => {}
                Event::ConfirmDownload(request) => {
                    self.pending_confirmations.push_back(request);
                }
            }
        }
    }

    fn handle_keys(&mut self, ctx: &egui::Context) {
        let typing = ctx.memory(|m| m.focused().is_some());
        let modal_open = self.add.is_some() || self.settings.is_some() || self.delete.is_some();
        let events = ctx.input(|i| i.events.clone());
        for e in events {
            match e {
                egui::Event::Paste(text) if !typing && !modal_open => {
                    let urls: Vec<&str> = text
                        .split_whitespace()
                        .filter(|u| u.starts_with("http://") || u.starts_with("https://"))
                        .collect();
                    if urls.len() == 1 {
                        self.add = Some(AddDialog::new(&self.mgr, urls[0]));
                    } else if urls.len() > 1 {
                        for u in &urls {
                            self.pending_confirmations
                                .push_back(DownloadRequest::from_url(u.to_string()));
                        }
                        self.toast(format!("{} liens à confirmer", urls.len()));
                    }
                    if !urls.is_empty() {
                        // This paste has already seeded the dialog/queue. Do not replay
                        // it into the newly focused URL field in the same frame.
                        ctx.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Paste(_))));
                    }
                }
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } if !typing && !modal_open => match key {
                    egui::Key::N if modifiers.command => {
                        self.add = Some(AddDialog::new(&self.mgr, ""))
                    }
                    egui::Key::Comma if modifiers.command => {
                        self.settings = Some(SettingsDialog::new(&self.mgr, 0))
                    }
                    egui::Key::Delete | egui::Key::Backspace => {
                        if let Some(id) = self.selected {
                            self.delete = Some(DeleteDialog::new(vec![id]));
                        }
                    }
                    egui::Key::Space => {
                        if let Some(d) = self.selected_dl() {
                            if lock(&d).running() {
                                self.mgr.stop(&d)
                            } else {
                                self.mgr.start(&d)
                            }
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }

    // ---------------------------------------------------------------- sidebar
    fn sidebar(&mut self, ui: &mut egui::Ui, p: &Palette) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            if let Some(t) = &self.logo {
                ui.add(egui::Image::new(t).fit_to_exact_size(vec2(32.0, 32.0)));
            }
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(egui::RichText::new("Open Transfer").font(theme::bold(14.5)));
                ui.label(egui::RichText::new("Manager").size(12.0).color(p.muted));
            });
        });
        ui.add_space(14.0);
        let bottom_h = 132.0;
        let nav_h = (ui.available_height() - bottom_h).max(80.0);
        egui::ScrollArea::vertical()
            .id_salt("nav")
            .max_height(nav_h)
            .auto_shrink([false, true])
            .show(ui, |ui| self.nav_lists(ui, p));
        ui.add_space((ui.available_height() - bottom_h + 18.0).max(4.0));
        self.sidebar_bottom(ui, p);
    }

    fn nav_lists(&mut self, ui: &mut egui::Ui, p: &Palette) {
        let all = self.mgr.list();
        let count = |f: &dyn Fn(&Download) -> bool| all.iter().filter(|d| f(&lock(d))).count();
        section(ui, p, "LIBRARY");
        let items: Vec<(View, Icon, Option<Color32>, usize)> = vec![
            (View::All, Icon::List, None, all.len()),
            (View::Active, Icon::Download, None, count(&|d| d.running())),
            (
                View::Paused,
                Icon::Clock,
                None,
                count(&|d| !d.running() && d.status == Status::Stopped),
            ),
            (
                View::Done,
                Icon::Check,
                None,
                count(&|d| d.status == Status::Complete),
            ),
            (
                View::Failed,
                Icon::Alert,
                None,
                count(&|d| d.status == Status::Error),
            ),
        ];
        for (v, icon, color, n) in items {
            self.nav_item(ui, p, v, icon, color, n);
        }
        ui.add_space(10.0);
        section(ui, p, "CATEGORIES");
        let cats: Vec<String> = self
            .mgr
            .settings()
            .categories
            .iter()
            .map(|c| c.name.clone())
            .collect();
        for c in cats {
            let (icon, color) = match c.as_str() {
                "Video" => (Icon::Video, theme::type_color(FileType::Video)),
                "Music" => (Icon::Music, theme::type_color(FileType::Audio)),
                "Images" => (Icon::Image, theme::type_color(FileType::Image)),
                "Documents" => (Icon::Doc, theme::type_color(FileType::Document)),
                "Archives" => (Icon::Archive, theme::type_color(FileType::Archive)),
                "Programs" => (Icon::Program, theme::type_color(FileType::Program)),
                _ => (Icon::File, p.muted),
            };
            let n = count(&|d| d.category == c);
            self.nav_item(ui, p, View::Category(c), icon, Some(color), n);
        }
    }

    fn sidebar_bottom(&mut self, ui: &mut egui::Ui, p: &Palette) {
        // browser bridge status (refreshed every few seconds, not every frame)
        if self.bridge.0.elapsed() > Duration::from_secs(5) {
            let st = crate::browsers::status();
            self.bridge = (
                Instant::now(),
                st.iter()
                    .filter(|(_, _, r)| *r)
                    .map(|(n, _, _)| n.clone())
                    .collect(),
            );
        }
        let connected = self.bridge.1.clone();
        widgets::card(p)
            .inner_margin(egui::Margin::symmetric(12, 10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 3.0;
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
                    ui.painter().circle_filled(
                        r.center(),
                        4.0,
                        if connected.is_empty() {
                            p.warn
                        } else {
                            p.success
                        },
                    );
                    ui.label(
                        egui::RichText::new(if connected.is_empty() {
                            "Browser bridge not registered"
                        } else {
                            "Browser bridge registered"
                        })
                        .size(12.0),
                    );
                });
                let txt = if connected.is_empty() {
                    "Connect in Settings → Browsers".to_string()
                } else {
                    connected.join(" · ")
                };
                let resp = ui.add(
                    egui::Label::new(egui::RichText::new(txt).size(11.5).color(p.muted))
                        .truncate()
                        .sense(Sense::click()),
                );
                if resp.clicked() {
                    self.settings = Some(SettingsDialog::new(&self.mgr, 5));
                }
            });
        ui.add_space(8.0);
        {
            // theme switch: light / dark / system
            let mut mode = self.mgr.settings().theme;
            let (rect, _) =
                ui.allocate_exact_size(vec2(ui.available_width(), 38.0), Sense::hover());
            ui.painter().rect_filled(rect, 10, p.alt);
            let w = (rect.width() - 6.0) / 3.0;
            for (i, (m, icon, tip)) in [
                (ThemeMode::Light, Icon::Sun, "Light theme"),
                (ThemeMode::Dark, Icon::Moon, "Dark theme"),
                (ThemeMode::System, Icon::Monitor, "Follow system"),
            ]
            .into_iter()
            .enumerate()
            {
                let r = Rect::from_min_size(
                    pos2(rect.min.x + 3.0 + i as f32 * w, rect.min.y + 3.0),
                    vec2(w, 32.0),
                );
                let resp = ui
                    .interact(r, ui.scope_id().with(("theme", i)), Sense::click())
                    .on_hover_text(tip);
                let on = mode == m;
                if on {
                    ui.painter().rect_filled(r, 8, p.surf);
                } else if resp.hovered() {
                    ui.painter().rect_filled(r, 8, p.hover);
                }
                icons::paint(
                    ui.painter(),
                    Rect::from_center_size(r.center(), vec2(16.0, 16.0)),
                    icon,
                    if on { p.text } else { p.muted },
                );
                if resp.clicked() {
                    mode = m;
                    self.mgr.update_settings(|s| s.theme = m);
                }
            }
        }
    }

    fn nav_item(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        v: View,
        icon: Icon,
        color: Option<Color32>,
        n: usize,
    ) {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
        let on = self.view == v;
        if on {
            ui.painter().rect_filled(rect, 8, p.soft);
        } else if resp.hovered() {
            ui.painter().rect_filled(rect, 8, p.hover);
        }
        let fg = if on {
            if p.dark { Color32::WHITE } else { p.accent }
        } else {
            p.text
        };
        icons::paint(
            ui.painter(),
            Rect::from_min_size(
                pos2(rect.min.x + 10.0, rect.center().y - 9.0),
                vec2(18.0, 18.0),
            ),
            icon,
            color.unwrap_or(fg),
        );
        ui.painter().text(
            pos2(rect.min.x + 38.0, rect.center().y),
            Align2::LEFT_CENTER,
            v.title(),
            if on {
                theme::bold(13.5)
            } else {
                theme::font(13.5)
            },
            fg,
        );
        if n > 0 {
            ui.painter().text(
                pos2(rect.max.x - 10.0, rect.center().y),
                Align2::RIGHT_CENTER,
                n.to_string(),
                theme::font(12.0),
                p.muted,
            );
        }
        if resp.clicked() {
            self.view = v;
        }
    }

    // ---------------------------------------------------------------- main area
    fn header(&mut self, ui: &mut egui::Ui, p: &Palette, visible: usize) {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.label(egui::RichText::new(self.view.title()).font(theme::bold(22.0)));
                let active = self.mgr.active_count();
                let mut sub = format!("{visible} download{}", if visible == 1 { "" } else { "s" });
                if active > 0 {
                    sub += &format!(
                        " · {active} active · {}",
                        fmt_rate(Some(self.mgr.total_rate()))
                    );
                }
                ui.label(egui::RichText::new(sub).size(12.0).color(p.muted));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::primary(ui, p, "New download", Some(Icon::Plus)).clicked() {
                    self.add = Some(AddDialog::new(&self.mgr, ""));
                }
                ui.add_space(4.0);
                widgets::text_input(ui, p, &mut self.search, "Search downloads", 200.0);
            });
        });
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, p: &Palette) {
        let sel = self.selected_dl();
        let (running, done) = sel
            .as_ref()
            .map(|d| {
                let d = lock(d);
                (d.running(), d.status == Status::Complete)
            })
            .unwrap_or((false, false));
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if widgets::icon_button(
                ui,
                p,
                Icon::Play,
                "Resume (Space)",
                sel.is_some() && !running && !done,
            )
            .clicked()
                && let Some(d) = &sel
            {
                self.mgr.start(d);
            }
            if widgets::icon_button(ui, p, Icon::Pause, "Pause (Space)", running).clicked()
                && let Some(d) = &sel
            {
                self.mgr.stop(d);
            }
            if widgets::icon_button(ui, p, Icon::Trash, "Remove (Del)", sel.is_some()).clicked()
                && let Some(id) = self.selected
            {
                self.delete = Some(DeleteDialog::new(vec![id]));
            }
            let (r, _) = ui.allocate_exact_size(vec2(13.0, 20.0), Sense::hover());
            ui.painter().line_segment(
                [pos2(r.center().x, r.min.y), pos2(r.center().x, r.max.y)],
                Stroke::new(1.0, p.border),
            );
            if widgets::secondary(ui, p, "Pause all", Some(Icon::PauseCircle)).clicked() {
                self.mgr.stop_all();
            }
            let q = self.mgr.queue_running();
            if widgets::secondary(
                ui,
                p,
                if q { "Stop queue" } else { "Start queue" },
                Some(Icon::Queue),
            )
            .clicked()
            {
                if q {
                    self.mgr.stop_queue();
                } else {
                    if !self.mgr.list().iter().any(|d| lock(d).queued) {
                        self.toast("The queue is empty: right-click a download → Add to queue");
                    }
                    self.mgr.start_queue();
                }
            }
            if widgets::secondary(ui, p, "Clear completed", Some(Icon::Check)).clicked() {
                let ids: Vec<u64> = self
                    .mgr
                    .list()
                    .iter()
                    .filter(|d| lock(d).status == Status::Complete)
                    .map(|d| lock(d).id)
                    .collect();
                if !ids.is_empty() {
                    self.delete = Some(DeleteDialog::new(ids));
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let s = self.mgr.settings();
                let label = if s.limiter_on {
                    format!("Speed limit: {}/s", fmt_size(Some(s.limit_kb * 1024)))
                } else {
                    "Speed limit: off".into()
                };
                if widgets::secondary(ui, p, &label, Some(Icon::Gauge)).clicked() {
                    self.settings = Some(SettingsDialog::new(&self.mgr, 2));
                }
            });
        });
    }

    fn list(&mut self, ui: &mut egui::Ui, p: &Palette, rows: &[DlRef]) {
        let frame = egui::Frame::new()
            .fill(p.surf)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(12);
        frame.show(ui, |ui| {
            ui.set_min_height(ui.available_height().max(120.0));
            let w = ui.available_width();
            let (hr, _) = ui.allocate_exact_size(vec2(w, 34.0), Sense::hover());
            // columns from the right: status 130, eta 80, speed 90, progress 210 (room for the scrollbar)
            let cols = Columns::new(hr.min.x, hr.max.x - 10.0);
            let hp = ui.painter();
            let hf = theme::bold(11.0);
            hp.text(pos2(hr.min.x + 16.0, hr.center().y), Align2::LEFT_CENTER, "NAME", hf.clone(), p.muted);
            hp.text(pos2(cols.progress.0, hr.center().y), Align2::LEFT_CENTER, "PROGRESS", hf.clone(), p.muted);
            hp.text(pos2(cols.speed.1, hr.center().y), Align2::RIGHT_CENTER, "SPEED", hf.clone(), p.muted);
            hp.text(pos2(cols.eta.1, hr.center().y), Align2::RIGHT_CENTER, "TIME LEFT", hf.clone(), p.muted);
            hp.text(pos2(cols.status.0, hr.center().y), Align2::LEFT_CENTER, "STATUS", hf, p.muted);
            hp.line_segment([pos2(hr.min.x, hr.max.y), pos2(hr.max.x, hr.max.y)], Stroke::new(1.0, p.border));
            if rows.is_empty() {
                ui.add_space(60.0);
                ui.vertical_centered(|ui| {
                    ui.label(egui::RichText::new("No downloads here yet").font(theme::bold(15.0)));
                    ui.label(egui::RichText::new("Click “New download”, paste a link (Ctrl+V), or use the browser extension.").color(p.muted));
                });
                return;
            }
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                for dl in rows {
                    self.row(ui, p, dl, &cols);
                }
            });
        });
    }

    fn row(&mut self, ui: &mut egui::Ui, p: &Palette, dl: &DlRef, cols: &Columns) {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 58.0), Sense::click());
        if !ui.is_rect_visible(rect) {
            return;
        }
        let d = lock(dl);
        let gold = matches!(&d.kind, Kind::Media(o) if o.music_enriched);
        let id = d.id;
        let selected = self.selected == Some(id);
        let painter = ui.painter_at(rect);
        if selected {
            painter.rect_filled(rect, 0, p.sel);
        } else if resp.hovered() {
            painter.rect_filled(rect, 0, theme::tint(p.hover, 0.6));
        } else if gold {
            painter.rect_filled(rect, 0, theme::tint(theme::gold(p.dark), 0.07));
        }
        painter.line_segment(
            [
                pos2(rect.min.x, rect.max.y - 0.5),
                pos2(rect.max.x, rect.max.y - 0.5),
            ],
            Stroke::new(1.0, p.border),
        );
        // badge + name + subline
        let badge = Rect::from_min_size(
            pos2(rect.min.x + 16.0, rect.center().y - 20.0),
            vec2(40.0, 40.0),
        );
        let badge_name = if d.is_media() && extension(&d.file_name).is_empty() {
            match &d.kind {
                Kind::Media(o) if o.quality == "audio" => format!(
                    "x.{}",
                    if o.audio_format == "original" {
                        "m4a"
                    } else {
                        &o.audio_format
                    }
                ),
                _ => "x.mp4".into(),
            }
        } else {
            d.file_name.clone()
        };
        widgets::type_badge(&painter, badge, &badge_name, p.dark);
        let name_x = badge.max.x + 12.0;
        let name_w = cols.progress.0 - name_x - 16.0;
        let g = widgets::elided(
            &painter,
            d.display_name(),
            theme::font(13.5),
            if gold { theme::gold(p.dark) } else { p.text },
            name_w,
        );
        let name_y = if super::text::contains_arabic(d.display_name()) {
            rect.center().y + 1.0 - g.size().y
        } else {
            rect.center().y - 17.0
        };
        painter.galley(pos2(name_x, name_y), g, p.text);
        let mut sub = vec![];
        if let Some(s) = d.display_size() {
            sub.push(fmt_size(Some(s)));
        }
        sub.push(d.host());
        if let Kind::Media(o) = &d.kind {
            if gold {
                sub.push("MP3 · Pochette + paroles".into());
            }
            sub.push(if o.quality == "audio" {
                if o.audio_format == "original" {
                    "audio via yt-dlp".into()
                } else {
                    format!(
                        "{} {} kbps via yt-dlp",
                        o.audio_format.to_uppercase(),
                        o.audio_kbps
                    )
                }
            } else if o.quality == "best" {
                "best quality via yt-dlp".into()
            } else {
                format!("{}p via yt-dlp", o.quality)
            });
        }
        let g = widgets::elided(
            &painter,
            &sub.join(" · "),
            theme::font(12.0),
            p.muted,
            name_w,
        );
        painter.galley(pos2(name_x, rect.center().y + 3.0), g, p.muted);
        // progress
        let frac = d.progress();
        let bar = Rect::from_min_size(
            pos2(cols.progress.0, rect.center().y - 3.0),
            vec2(cols.progress.1 - cols.progress.0 - 46.0, 6.0),
        );
        let fill = match d.status {
            Status::Complete => Some(p.success),
            Status::Error => Some(p.danger),
            _ if d.running() => None,
            _ => Some(p.muted),
        };
        widgets::progress_bar(&painter, bar, frac.unwrap_or(0.0) as f32, p.track, fill);
        let pct = match frac {
            Some(f) => format!("{}%", (f * 100.0).floor() as u32),
            None if d.running() => "…".into(),
            None => String::new(),
        };
        painter.text(
            pos2(cols.progress.1, rect.center().y),
            Align2::RIGHT_CENTER,
            pct,
            theme::font(12.0),
            p.muted,
        );
        // speed / eta
        if d.running() {
            painter.text(
                pos2(cols.speed.1, rect.center().y),
                Align2::RIGHT_CENTER,
                fmt_rate(d.rt.rate),
                theme::font(13.0),
                p.text,
            );
            painter.text(
                pos2(cols.eta.1, rect.center().y),
                Align2::RIGHT_CENTER,
                fmt_eta(d.time_left()),
                theme::font(13.0),
                p.muted,
            );
        }
        // status pill
        let (fg, bg) = match d.status {
            Status::Complete => (p.success, theme::tint(p.success, 0.13)),
            Status::Error => (p.danger, theme::tint(p.danger, 0.13)),
            Status::Downloading | Status::Connecting => {
                (p.accent, theme::tint(p.btn, if p.dark { 0.2 } else { 0.1 }))
            }
            Status::Stopped => (p.muted, p.alt),
        };
        let label = match d.status {
            Status::Downloading => "Downloading".to_string(),
            _ => d.status_text(),
        };
        widgets::status_pill(
            &painter,
            pos2(cols.status.0, rect.center().y),
            &label,
            fg,
            bg,
        );
        let (running, complete, path, url, queued, save_dir) = (
            d.running(),
            d.status == Status::Complete,
            d.path(),
            d.url.clone(),
            d.queued,
            d.save_dir.clone(),
        );
        drop(d);

        if resp.clicked() {
            self.selected = Some(id);
        }
        if resp.double_clicked() {
            if complete {
                paths::open_path(&path);
            } else if !running {
                self.mgr.start(dl);
            }
        }
        resp.context_menu(|ui| {
            self.selected = Some(id);
            if complete && ui.button("Open").clicked() {
                paths::open_path(&path);
                ui.close();
            }
            if ui.button("Show in folder").clicked() {
                paths::reveal(if complete { &path } else { &save_dir });
                ui.close();
            }
            ui.separator();
            if !complete && !running && ui.button("Resume").clicked() {
                self.mgr.start(dl);
                ui.close();
            }
            if running && ui.button("Pause").clicked() {
                self.mgr.stop(dl);
                ui.close();
            }
            if ui.button("Download again").clicked() {
                self.mgr.redownload(dl);
                ui.close();
            }
            ui.separator();
            if !queued && ui.button("Add to queue").clicked() {
                lock(dl).queued = true;
                self.mgr.save();
                ui.close();
            }
            if queued && ui.button("Remove from queue").clicked() {
                lock(dl).queued = false;
                self.mgr.save();
                ui.close();
            }
            if ui.button("Copy link").clicked() {
                ui.ctx().copy_text(url.clone());
                ui.close();
            }
            ui.separator();
            if ui.button("Remove…").clicked() {
                self.delete = Some(DeleteDialog::new(vec![id]));
                ui.close();
            }
        });
    }

    fn details(&mut self, ui: &mut egui::Ui, p: &Palette) {
        let Some(dl) = self.selected_dl() else { return };
        widgets::card(p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let d = lock(&dl);
            let (running, complete, path, save_dir, url) = (
                d.running(),
                d.status == Status::Complete,
                d.path(),
                d.save_dir.clone(),
                d.url.clone(),
            );
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.set_max_width(ui.available_width() - 330.0);
                    ui.add(
                        egui::Label::new(super::text::job(
                            d.display_name(),
                            theme::bold(14.0),
                            p.text,
                        ))
                        .truncate(),
                    );
                    ui.add(
                        egui::Label::new(egui::RichText::new(&d.url).size(12.0).color(p.muted))
                            .truncate(),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::secondary(ui, p, "Copy link", Some(Icon::Link)).clicked() {
                        ui.ctx().copy_text(url.clone());
                        self_toast(&mut self.toast, "Link copied");
                    }
                    if widgets::secondary(ui, p, "Show in folder", Some(Icon::Folder)).clicked() {
                        paths::reveal(if complete { &path } else { &save_dir });
                    }
                    let (label, icon) = if complete {
                        ("Open", Icon::Play)
                    } else if running {
                        ("Pause", Icon::Pause)
                    } else {
                        ("Resume", Icon::Play)
                    };
                    if widgets::secondary(ui, p, label, Some(icon)).clicked() {
                        if complete {
                            paths::open_path(&path);
                        } else if running {
                            self.mgr.stop(&dl);
                        } else {
                            self.mgr.start(&dl);
                        }
                    }
                });
            });
            ui.add_space(4.0);
            // one block per connection part (http) or one bar (media)
            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 10.0), Sense::hover());
            let painter = ui.painter();
            if !d.is_media() && d.size.is_some() && !d.segments.is_empty() && !complete {
                let size = d.size.unwrap_or(1).max(1) as f32;
                painter.rect_filled(r, 3, p.track);
                for s in &d.segments {
                    let x0 = r.min.x + s.start as f32 / size * r.width();
                    let x1 = r.min.x + s.pos as f32 / size * r.width();
                    if x1 - x0 >= 0.5 {
                        let c = if running { theme::GRADIENT[1] } else { p.muted };
                        painter.rect_filled(
                            Rect::from_min_max(pos2(x0, r.min.y), pos2(x1, r.max.y)),
                            2,
                            c,
                        );
                    }
                }
            } else {
                let fill = if complete {
                    Some(p.success)
                } else if d.status == Status::Error {
                    Some(p.danger)
                } else if running {
                    None
                } else {
                    Some(p.muted)
                };
                widgets::progress_bar(
                    painter,
                    r,
                    d.progress().unwrap_or(0.0) as f32,
                    p.track,
                    fill,
                );
            }
            ui.add_space(2.0);
            let mut parts = Vec::new();
            let total = d.display_size();
            parts.push(match total {
                Some(t) => format!(
                    "{} of {}",
                    fmt_size(Some(d.downloaded())),
                    fmt_size(Some(t))
                ),
                None => fmt_size(Some(d.downloaded())),
            });
            if running {
                parts.push(fmt_rate(d.rt.rate));
                if let Some(t) = d.time_left() {
                    parts.push(format!("{} left", fmt_eta(Some(t))));
                }
                if !d.is_media() {
                    parts.push(format!(
                        "{} connection{}",
                        d.rt.connections,
                        if d.rt.connections == 1 { "" } else { "s" }
                    ));
                }
            }
            if !d.phase.is_empty() {
                parts.push(d.phase.clone());
            }
            if !d.is_media()
                && let Some(r) = d.resumable
            {
                parts.push(if r {
                    "resumable".into()
                } else {
                    "cannot resume".into()
                });
            }
            parts.push(format!(
                "Saved to {}",
                if complete {
                    path.display().to_string()
                } else {
                    save_dir.display().to_string()
                }
            ));
            ui.add(
                egui::Label::new(egui::RichText::new(parts.join("   ·   ")).size(12.5)).truncate(),
            );
            if d.status == Status::Error && !d.error.is_empty() {
                ui.label(egui::RichText::new(&d.error).size(12.5).color(p.danger));
            }
        });
    }

    fn status_bar(&self, ui: &mut egui::Ui, p: &Palette) {
        ui.horizontal(|ui| {
            let txt = match &self.toast {
                Some((t, at)) if at.elapsed() < Duration::from_secs(5) => t.clone(),
                _ => {
                    let a = self.mgr.active_count();
                    let mut s = if a > 0 {
                        format!("{a} active · ↓ {}", fmt_rate(Some(self.mgr.total_rate())))
                    } else {
                        "Ready".into()
                    };
                    if self.mgr.queue_running() {
                        s += " · queue running";
                    }
                    s
                }
            };
            ui.label(egui::RichText::new(txt).size(12.0).color(p.muted));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(format!("v{}", env!("CARGO_PKG_VERSION")))
                        .size(12.0)
                        .color(p.muted),
                );
                let yt = if *DEMO.lock().unwrap() {
                    Some("2026.10.26".to_string())
                } else {
                    YTDLP.get_or_init(media::ytdlp_version).clone()
                };
                ui.label(
                    egui::RichText::new(
                        yt.map(|v| format!("yt-dlp {v}"))
                            .unwrap_or("yt-dlp not installed".into()),
                    )
                    .size(12.0)
                    .color(p.muted),
                );
            });
        });
    }
}

static YTDLP: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();

fn self_toast(t: &mut Option<(String, Instant)>, s: &str) {
    *t = Some((s.to_string(), Instant::now()));
}

fn section(ui: &mut egui::Ui, p: &Palette, title: &str) {
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.add_space(10.0);
        ui.label(
            egui::RichText::new(title)
                .font(theme::bold(11.0))
                .color(p.muted),
        );
    });
}

struct Columns {
    progress: (f32, f32),
    speed: (f32, f32),
    eta: (f32, f32),
    status: (f32, f32),
}

impl Columns {
    /// Absolute x positions inside [left, right] (screen coordinates).
    fn new(_left: f32, right: f32) -> Self {
        let right = right - 16.0;
        let status = (right - 130.0, right);
        let eta = (status.0 - 92.0, status.0 - 12.0);
        let speed = (eta.0 - 90.0, eta.0);
        let progress = (speed.0 - 236.0, speed.0 - 26.0);
        Columns {
            progress,
            speed,
            eta,
            status,
        }
    }
}

impl eframe::App for OtmApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // theme
        let mode = self.mgr.settings().theme;
        let dark = match mode {
            ThemeMode::Dark => true,
            ThemeMode::Light => false,
            ThemeMode::System => ctx
                .system_theme()
                .map(|t| t == egui::Theme::Dark)
                .unwrap_or(true),
        };
        if dark != self.dark || ctx.cumulative_pass_nr() < 2 {
            self.dark = dark;
            theme::apply(&ctx, &theme::palette(dark));
        }
        let p = self.palette();
        if self.last_tick.elapsed() >= Duration::from_millis(500) && self.screenshot.is_none() {
            self.last_tick = Instant::now();
            self.mgr.tick();
        }
        for u in std::mem::take(&mut self.pending_urls) {
            self.pending_confirmations
                .push_back(DownloadRequest::from_url(u));
        }
        self.handle_events(&ctx);
        self.handle_keys(&ctx);
        ctx.request_repaint_after(if self.mgr.active_count() > 0 {
            Duration::from_millis(250)
        } else {
            Duration::from_secs(1)
        });

        egui::Panel::left("sidebar")
            .exact_size(236.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(p.side)
                    .inner_margin(egui::Margin {
                        left: 12,
                        right: 12,
                        top: 16,
                        bottom: 12,
                    })
                    .stroke(Stroke::new(1.0, p.border)),
            )
            .show(ui, |ui| self.sidebar(ui, &p));

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(p.bg).inner_margin(egui::Margin {
                left: 22,
                right: 22,
                top: 18,
                bottom: 10,
            }))
            .show(ui, |ui| {
                let rows = self.visible();
                self.header(ui, &p, rows.len());
                ui.add_space(4.0);
                self.toolbar(ui, &p);
                ui.add_space(4.0);
                let show_details = self.mgr.settings().show_details && self.selected_dl().is_some();
                let bottom = 26.0 + if show_details { 150.0 } else { 0.0 };
                let list_h = (ui.available_height() - bottom).max(160.0);
                ui.allocate_ui(vec2(ui.available_width(), list_h), |ui| {
                    self.list(ui, &p, &rows)
                });
                if show_details {
                    ui.add_space(8.0);
                    self.details(ui, &p);
                }
                ui.add_space(6.0);
                self.status_bar(ui, &p);
            });

        // Serialize external requests: never replace an existing modal or start
        // a transfer before the user explicitly confirms it.
        if self.add.is_none()
            && self.settings.is_none()
            && self.delete.is_none()
            && let Some(request) = self.pending_confirmations.pop_front()
        {
            self.add = Some(AddDialog::from_request(&self.mgr, request));
        }
        // dialogs
        if let Some(mut d) = self.add.take()
            && d.show(&ctx, &self.mgr, &p)
        {
            self.add = Some(d);
        }
        if let Some(mut d) = self.settings.take()
            && d.show(&ctx, &self.mgr, &p, self.logo.as_ref())
        {
            self.settings = Some(d);
        }
        if let Some(mut d) = self.delete.take() {
            match d.show(&ctx, &self.mgr, &p) {
                Some(true) => {
                    self.selected = None;
                }
                Some(false) => {}
                None => self.delete = Some(d),
            }
        }

        // screenshot mode (for docs and visual checks)
        if let Some((path, frames)) = &mut self.screenshot {
            *frames += 1;
            if *frames == 3 {
                self.selected = self.mgr.list().get(6).map(|d| lock(d).id);
            }
            let ready = self.add.as_ref().is_none_or(|a| !a.probing());
            if (*frames >= 30 && ready && *frames < 100_000) || *frames == 3000 {
                *frames = 100_000;
                let path = path.clone();
                let c2 = ctx.clone();
                ctx.request_screenshot(move |img| {
                    let _ = image::RgbaImage::from_raw(
                        img.size[0] as u32,
                        img.size[1] as u32,
                        img.as_raw().to_vec(),
                    )
                    .map(|i| i.save(&path));
                    c2.send_viewport_cmd(egui::ViewportCommand::Close);
                });
            }
            ctx.request_repaint();
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(a) = &self.api {
            a.stop();
        }
    }
}

/// Sample rows for screenshots (never saved).
fn demo_downloads(mgr: &Manager) {
    let base = mgr.settings().base_folder;
    type DemoRow<'a> = (
        &'a str,
        &'a str,
        &'a str,
        u64,
        f64,
        Status,
        Option<f64>,
        bool,
    );
    let rows: [DemoRow<'_>; 9] = [
        (
            "vacation-photos.zip",
            "Archives",
            "https://drive.example.com/vacation-photos.zip",
            48_312_445,
            1.0,
            Status::Complete,
            None,
            false,
        ),
        (
            "open-transfer-manager_0.2.0_amd64.deb",
            "Programs",
            "https://github.com/OthmenBedoui/open-transfer-manager/releases",
            9_400_000,
            1.0,
            Status::Complete,
            None,
            false,
        ),
        (
            "wallpaper-aurora-4k.png",
            "Images",
            "https://images.example.com/wallpaper-aurora-4k.png",
            12_582_912,
            1.0,
            Status::Complete,
            None,
            false,
        ),
        (
            "dataset-archive.7z",
            "Archives",
            "https://mirror.example.net/dataset-archive.7z",
            1_288_490_188,
            0.0,
            Status::Error,
            None,
            false,
        ),
        (
            "تقرير العمل 2026.pdf",
            "Documents",
            "https://example.org/annual-report-2026.pdf",
            2_841_002,
            1.0,
            Status::Complete,
            None,
            false,
        ),
        (
            "lofi beats - live mix.mp3",
            "Music",
            "https://soundcloud.com/lofi/live-mix",
            58_720_256,
            1.0,
            Status::Complete,
            None,
            true,
        ),
        (
            "blender-5.1-linux-x64.tar.xz",
            "Archives",
            "https://download.blender.org/release/blender-5.1-linux-x64.tar.xz",
            412_517_120,
            0.4532,
            Status::Stopped,
            None,
            false,
        ),
        (
            "Big Buck Bunny 4K [aqz-KE-bpKQ].mp4",
            "Video",
            "https://www.youtube.com/watch?v=aqz-KE-bpKQ",
            734_003_200,
            0.31,
            Status::Downloading,
            Some(4_100_000.0),
            true,
        ),
        (
            "Ubuntu 26.04 Desktop amd64.iso",
            "Archives",
            "https://releases.ubuntu.com/26.04/ubuntu-26.04-desktop-amd64.iso",
            6_012_000_000,
            0.62,
            Status::Downloading,
            Some(9_400_000.0),
            false,
        ),
    ];
    for (i, (name, cat, url, size, frac, status, rate, media)) in rows.into_iter().enumerate() {
        let kind = if media {
            Kind::Media(if cat == "Music" {
                MediaOpts {
                    quality: "audio".into(),
                    audio_kbps: 320,
                    music_enriched: true,
                    ..MediaOpts::default()
                }
            } else {
                MediaOpts {
                    quality: "1080".into(),
                    ..MediaOpts::default()
                }
            })
        } else {
            Kind::Http
        };
        let dl = if media {
            mgr.add_media(
                url,
                match &kind {
                    Kind::Media(o) => o.clone(),
                    _ => unreachable!(),
                },
                Some(base.join(cat)),
                name,
                false,
                false,
                vec![],
            )
        } else {
            mgr.add_http(url, name, &base.join(cat), cat, false, false, vec![])
        };
        let mut d = lock(&dl);
        d.file_name = name.to_string();
        d.size = Some(size);
        d.resumable = Some(true);
        d.status = status;
        d.date_added = now() - 3600.0 * (9 - i) as f64;
        if media {
            d.media_total = Some(size);
            d.media_done = (size as f64 * frac) as u64;
            if status == Status::Downloading {
                d.phase = "video".into();
                d.media_eta = Some(123.0);
            }
        } else {
            let half = size / 2;
            d.segments = (0..8)
                .map(|k| {
                    let a = k * size / 8;
                    let e = (k + 1) * size / 8 - 1;
                    let f = [1.0, 0.92, 0.8, 0.7, 0.55, 0.45, 0.32, 0.25][k as usize] * frac / 0.6;
                    let mut s = Segment::new(a, Some(e));
                    s.pos = a + ((e - a) as f64 * f.min(1.0)) as u64;
                    s
                })
                .collect();
            if status != Status::Downloading {
                d.segments = vec![{
                    let mut s = Segment::new(0, Some(size - 1));
                    s.pos = (size as f64 * frac) as u64;
                    s
                }];
            }
            let _ = half;
        }
        if status == Status::Error {
            d.error = "Server answered 403 (access denied — the link may have expired)".into();
        }
        if let Some(r) = rate {
            d.rt.running = true;
            d.rt.rate = Some(r);
            d.rt.connections = if media { 1 } else { 8 };
        }
    }
}

#[allow(dead_code)]
fn unused(_: StrokeKind) {}
