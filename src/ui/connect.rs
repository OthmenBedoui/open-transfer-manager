//! "Connectors" page: the drives and servers files can be sent to, and the form to add,
//! sign in to, or edit one.

use super::app::{OtmApp, View};
use super::icons::Icon;
use super::send::{engine_banner, mono_badge};
use super::theme::{self, Palette};
use super::widgets;
use crate::connectors::{Connector, Kind, SftpAuth};
use crate::secrets;
use crate::uploads::Uploads;
use egui::{Color32, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};

type TestResult = Result<String, String>;

/// A token, plus the OneDrive drive (id, type) found right after sign-in.
type SignInResult = Result<(String, Option<(String, String)>), String>;

enum SignInMsg {
    /// The page to open by hand if the browser did not appear.
    Link(String),
    Done(SignInResult),
}

/// A browser sign-in in progress (`rclone authorize`), cancelled if the form closes.
struct SignIn {
    rx: mpsc::Receiver<SignInMsg>,
    cancel: Arc<AtomicBool>,
    link: Option<String>,
}

impl Drop for SignIn {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
}

impl SignIn {
    fn start(kind: Kind, ctx: egui::Context) -> Self {
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        std::thread::spawn(move || {
            let link_tx = tx.clone();
            let link_ctx = ctx.clone();
            let r = crate::rclone::authorize(kind, &stop, |url| {
                let _ = link_tx.send(SignInMsg::Link(url));
                link_ctx.request_repaint();
            })
            .and_then(|token| {
                let drive = if kind == Kind::Onedrive {
                    Some(crate::rclone::onedrive_drive(&token)?)
                } else {
                    None
                };
                Ok((token, drive))
            });
            let _ = tx.send(SignInMsg::Done(r));
            ctx.request_repaint();
        });
        SignIn {
            rx,
            cancel,
            link: None,
        }
    }
}

pub struct Editor {
    pub form: Connector,
    port: String,
    secret: String,
    secret_saved: bool,
    testing: Option<mpsc::Receiver<TestResult>>,
    result: Option<TestResult>,
    confirm_delete: bool,
    signin: Option<SignIn>,
}

impl Editor {
    fn new(form: Connector, secret_saved: bool) -> Self {
        Editor {
            port: form.port.map(|p| p.to_string()).unwrap_or_default(),
            form,
            secret: String::new(),
            secret_saved,
            testing: None,
            result: None,
            confirm_delete: false,
            signin: None,
        }
    }

    pub fn for_demo(c: Connector) -> Self {
        let mut e = Editor::new(c, false);
        e.result = Some(Ok("Connected · 12 items in the folder · 41 GB free".into()));
        e
    }

    /// The form with the port field applied.
    fn connector(&self) -> Connector {
        let mut c = self.form.clone();
        c.port = self.port.trim().parse().ok();
        c
    }

    /// The secret to use: typed now, saved earlier (`None` = read the keychain), or none.
    fn secret_arg(&self) -> Option<String> {
        if !self.secret.is_empty() {
            Some(self.secret.clone())
        } else if self.secret_saved {
            None
        } else {
            Some(String::new())
        }
    }
}

#[derive(Default)]
pub struct ConnectState {
    pub editor: Option<Editor>,
}

fn field(
    ui: &mut egui::Ui,
    p: &Palette,
    label: &str,
    value: &mut String,
    hint: &str,
    password: bool,
) -> bool {
    ui.label(egui::RichText::new(label).size(12.0).color(p.muted));
    let r = egui::Frame::new()
        .fill(if p.dark {
            Color32::from_rgb(0x0F, 0x11, 0x17)
        } else {
            Color32::WHITE
        })
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(10, 7))
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::singleline(value)
                    .hint_text(hint)
                    .password(password)
                    .frame(egui::Frame::NONE)
                    .desired_width(ui.available_width())
                    .margin(egui::Margin::ZERO),
            )
        })
        .inner;
    ui.add_space(4.0);
    r.changed()
}

impl OtmApp {
    pub(super) fn connectors_view(&mut self, ui: &mut egui::Ui, p: &Palette) {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.label(egui::RichText::new("Connectors").font(theme::bold(22.0)));
                ui.label(
                    egui::RichText::new("Connect once, then send from the Send page or “Send to…” on a finished download.")
                        .size(12.0)
                        .color(p.muted),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::secondary(ui, p, "Go to Send", Some(Icon::Upload)).clicked() {
                    self.view = View::Send;
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
        let right_w = (w * 0.42).clamp(290.0, 360.0);
        let h = ui.available_height();
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                vec2(w - right_w - 16.0, h),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("conn-list")
                        .auto_shrink([false, false])
                        .show(ui, |ui| self.connector_cards(ui, p));
                },
            );
            ui.add_space(16.0);
            ui.allocate_ui_with_layout(
                vec2(right_w, h),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("conn-form")
                        .auto_shrink([false, false])
                        .show(ui, |ui| self.editor_panel(ui, p));
                },
            );
        });
    }

    /// A clickable card; returns true when clicked.
    fn card_tile(
        ui: &mut egui::Ui,
        p: &Palette,
        kind: Kind,
        title: &str,
        sub: &str,
        pill: (&str, Color32),
        selected: bool,
    ) -> bool {
        let (rect, resp) = ui.allocate_exact_size(vec2(196.0, 96.0), Sense::click());
        let bg = if selected {
            theme::tint(p.btn, if p.dark { 0.12 } else { 0.07 })
        } else if resp.hovered() {
            p.hover
        } else {
            p.surf
        };
        ui.painter().rect_filled(rect, 12, bg);
        ui.painter().rect_stroke(
            rect,
            12,
            Stroke::new(1.0, if selected { p.btn } else { p.border }),
            StrokeKind::Inside,
        );
        let badge = Rect::from_min_size(rect.min + vec2(11.0, 11.0), vec2(30.0, 30.0));
        mono_badge(ui.painter(), badge, kind.mono(), kind, p.dark);
        let g = widgets::elided(
            ui.painter(),
            title,
            theme::bold(13.0),
            p.text,
            rect.width() - 62.0,
        );
        ui.painter().galley(
            pos2(rect.min.x + 50.0, badge.center().y - g.size().y / 2.0),
            g,
            p.text,
        );
        let g = widgets::elided(
            ui.painter(),
            sub,
            theme::font(11.5),
            p.muted,
            rect.width() - 22.0,
        );
        ui.painter()
            .galley(pos2(rect.min.x + 11.0, rect.min.y + 50.0), g, p.muted);
        widgets::status_pill(
            ui.painter(),
            pos2(rect.min.x + 10.0, rect.max.y - 17.0),
            pill.0,
            pill.1,
            theme::tint(pill.1, if p.dark { 0.14 } else { 0.10 }),
        );
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
    }

    fn connector_cards(&mut self, ui: &mut egui::Ui, p: &Palette) {
        let conns = self.uploads.connectors();
        let editing_id = self.connect.editor.as_ref().map(|e| e.form.id);
        let editing_kind = self
            .connect
            .editor
            .as_ref()
            .filter(|e| e.form.id == 0)
            .map(|e| e.form.kind);
        if !conns.is_empty() {
            heading(ui, p, "YOUR DESTINATIONS", "");
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                for c in &conns {
                    if Self::card_tile(
                        ui,
                        p,
                        c.kind,
                        &c.display_name(),
                        &c.address(),
                        ("● Ready", p.success),
                        editing_id == Some(c.id),
                    ) {
                        let saved = self.uploads.has_secret(c);
                        self.connect.editor = Some(Editor::new(c.clone(), saved));
                    }
                }
            });
            ui.add_space(14.0);
        }
        for (title, hint, kinds) in [
            (
                "CLOUD DRIVES",
                "sign in on the provider's own page",
                &Kind::DRIVES[..],
            ),
            (
                "SERVERS",
                "for developers and self-hosting",
                &Kind::SERVERS[..],
            ),
        ] {
            heading(ui, p, title, hint);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                for &k in kinds {
                    if Self::card_tile(
                        ui,
                        p,
                        k,
                        k.label(),
                        k.blurb(),
                        ("+ Connect", p.text),
                        editing_kind == Some(k),
                    ) {
                        self.connect.editor = Some(Editor::new(Connector::new(k), false));
                    }
                }
            });
            ui.add_space(14.0);
        }
        heading(ui, p, "COMING NEXT", "");
        widgets::card(p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 4.0;
            for (title, body) in [
                ("Communities", "Telegram, Discord, PeerTube, Mastodon, then YouTube with your own API key."),
                ("Relay", "Send a link straight from the web to your drives, without keeping a copy on this computer."),
            ] {
                ui.label(egui::RichText::new(title).font(theme::bold(13.0)));
                ui.label(egui::RichText::new(body).size(12.0).color(p.muted));
                ui.add_space(4.0);
            }
        });
    }

    fn editor_panel(&mut self, ui: &mut egui::Ui, p: &Palette) {
        let uploads = self.uploads.clone();
        let mut toast = None;
        let mut close = false;
        widgets::card(p).inner_margin(egui::Margin::same(16)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let Some(ed) = self.connect.editor.as_mut() else {
                ui.label(egui::RichText::new("Add a destination").font(theme::bold(15.0)));
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new("Pick a drive or server type on the left, or one of your destinations to edit it.")
                        .size(12.5)
                        .color(p.muted),
                );
                ui.add_space(10.0);
                ui.label(
                    egui::RichText::new(format!("Passwords and keys are kept in {}, never in OTM's settings files.", secrets::backend()))
                        .size(12.0)
                        .color(p.muted),
                );
                return;
            };
            if let Some(rx) = &ed.testing
                && let Ok(r) = rx.try_recv()
            {
                ed.result = Some(r);
                ed.testing = None;
            }
            // browser sign-in progress
            let mut signed_in = None;
            if let Some(si) = &mut ed.signin {
                while let Ok(m) = si.rx.try_recv() {
                    match m {
                        SignInMsg::Link(l) => si.link = Some(l),
                        SignInMsg::Done(r) => signed_in = Some(r),
                    }
                }
            }
            if let Some(r) = signed_in {
                ed.signin = None;
                match r {
                    Ok((token, drive)) => {
                        if let Some((id, t)) = drive {
                            ed.form.drive_id = id;
                            ed.form.drive_type = t;
                        }
                        match uploads.save_connector(ed.connector(), Some(&token)) {
                            Ok(id) => {
                                toast = Some(format!("Signed in to {}", ed.form.kind.label()));
                                if let Some(c) = uploads.connector(id) {
                                    *ed = Editor::new(c.clone(), true);
                                    ed.testing = Some(Uploads::test(c, None));
                                }
                            }
                            Err(e) => ed.result = Some(Err(e)),
                        }
                    }
                    Err(e) => ed.result = Some(Err(e)),
                }
            }
            let kind = ed.form.kind;
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::hover());
                mono_badge(ui.painter(), r, kind.mono(), kind, p.dark);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    let title = if ed.form.id == 0 { format!("New {} connection", kind.label()) } else { ed.form.display_name() };
                    ui.add(egui::Label::new(egui::RichText::new(title).font(theme::bold(15.0))).truncate());
                    ui.label(egui::RichText::new(if ed.form.id == 0 { "Saved on this computer" } else { "Edit connection" }).size(12.0).color(p.muted));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    if widgets::icon_button(ui, p, Icon::Close, "Close", true).clicked() {
                        close = true;
                    }
                });
            });
            ui.add_space(8.0);
            let mut changed = false;
            changed |= field(ui, p, "Name", &mut ed.form.name, "Shown in the Send list", false);
            let saved_hint = "Saved · leave empty to keep it";
            match kind {
                Kind::Sftp | Kind::Ftp => {
                    changed |= field(ui, p, "Host", &mut ed.form.host, "sftp.example.com or 203.0.113.10", false);
                    ui.columns(2, |cols| {
                        changed |= field(&mut cols[0], p, "Port", &mut ed.port, &kind.default_port().unwrap_or(22).to_string(), false);
                        changed |= field(&mut cols[1], p, "User", &mut ed.form.user, if kind == Kind::Ftp { "anonymous" } else { "deploy" }, false);
                    });
                    if kind == Kind::Sftp {
                        ui.label(egui::RichText::new("Sign in with").size(12.0).color(p.muted));
                        changed |= widgets::segmented(
                            ui,
                            p,
                            &mut ed.form.auth,
                            &[(SftpAuth::Agent, "SSH agent"), (SftpAuth::KeyFile, "Key file"), (SftpAuth::Password, "Password")],
                            ((ui.available_width() - 8.0) / 3.0).max(70.0),
                        );
                        ui.add_space(6.0);
                        match ed.form.auth {
                            SftpAuth::Agent => {
                                ui.label(
                                    egui::RichText::new("Uses the keys loaded in your SSH agent (ssh-add, Pageant, 1Password…).")
                                        .size(12.0)
                                        .color(p.muted),
                                );
                                ui.add_space(4.0);
                            }
                            SftpAuth::KeyFile => {
                                changed |= field(ui, p, "Private key file", &mut ed.form.key_file, "~/.ssh/id_ed25519", false);
                                if ui.add(egui::Label::new(egui::RichText::new("Choose a key file…").size(12.0).color(p.accent)).sense(Sense::click())).clicked()
                                    && let Some(f) = rfd::FileDialog::new().set_directory(crate::paths::home().join(".ssh")).pick_file()
                                {
                                    ed.form.key_file = f.display().to_string();
                                    changed = true;
                                }
                                ui.add_space(4.0);
                                changed |= field(ui, p, "Key passphrase (if any)", &mut ed.secret, if ed.secret_saved { saved_hint } else { "" }, true);
                            }
                            SftpAuth::Password => {
                                changed |= field(ui, p, "Password", &mut ed.secret, if ed.secret_saved { saved_hint } else { "" }, true);
                            }
                        }
                    } else {
                        changed |= field(ui, p, "Password", &mut ed.secret, if ed.secret_saved { saved_hint } else { "" }, true);
                        changed |= ui.checkbox(&mut ed.form.tls, "Use TLS (FTPS)").changed();
                        ui.add_space(4.0);
                    }
                    changed |= field(ui, p, "Remote folder", &mut ed.form.folder, "/var/www/releases (empty: home folder)", false);
                }
                Kind::Webdav => {
                    changed |= field(ui, p, "Address", &mut ed.form.url, "https://cloud.example.com/remote.php/dav/files/me", false);
                    ui.label(egui::RichText::new("Server").size(12.0).color(p.muted));
                    changed |= widgets::segmented(
                        ui,
                        p,
                        &mut ed.form.vendor,
                        &[("nextcloud".to_string(), "Nextcloud"), ("owncloud".to_string(), "ownCloud"), ("other".to_string(), "Other")],
                        ((ui.available_width() - 8.0) / 3.0).max(70.0),
                    );
                    ui.add_space(6.0);
                    changed |= field(ui, p, "User", &mut ed.form.user, "", false);
                    changed |= field(ui, p, kind.secret_label(), &mut ed.secret, if ed.secret_saved { saved_hint } else { "" }, true);
                    changed |= field(ui, p, "Folder", &mut ed.form.folder, "Uploads (empty: top folder)", false);
                }
                Kind::S3 => {
                    changed |= field(ui, p, "Endpoint", &mut ed.form.endpoint, "Empty for AWS · https://s3.eu-central-003.backblazeb2.com", false);
                    changed |= field(ui, p, "Region (optional)", &mut ed.form.region, "us-east-1", false);
                    changed |= field(ui, p, "Access key", &mut ed.form.access_key, "", false);
                    changed |= field(ui, p, "Secret key", &mut ed.secret, if ed.secret_saved { saved_hint } else { "" }, true);
                    changed |= field(ui, p, "Bucket and folder", &mut ed.form.folder, "my-bucket/releases", false);
                }
                Kind::Mega => {
                    changed |= field(ui, p, "Email", &mut ed.form.user, "you@example.com", false);
                    changed |= field(ui, p, "Password", &mut ed.secret, if ed.secret_saved { saved_hint } else { "" }, true);
                    changed |= field(ui, p, "Folder", &mut ed.form.folder, "OTM", false);
                }
                _ => {
                    if kind == Kind::Pcloud {
                        ui.label(egui::RichText::new("Data region").size(12.0).color(p.muted));
                        changed |= widgets::segmented(
                            ui,
                            p,
                            &mut ed.form.region,
                            &[("us".to_string(), "United States"), ("eu".to_string(), "Europe")],
                            ((ui.available_width() - 8.0) / 2.0).max(90.0),
                        );
                        ui.add_space(6.0);
                    }
                    changed |= field(ui, p, "Folder", &mut ed.form.folder, "OTM (empty: top folder)", false);
                    let signed = ed.secret_saved || !ed.secret.is_empty();
                    ui.add_space(2.0);
                    let mut cancel_signin = false;
                    if let Some(si) = &ed.signin {
                        ui.horizontal(|ui| {
                            ui.add(egui::Spinner::new());
                            ui.label(egui::RichText::new("Finish signing in in your browser…").size(12.5).color(p.muted));
                        });
                        ui.horizontal_wrapped(|ui| {
                            if let Some(link) = &si.link
                                && ui.add(egui::Label::new(egui::RichText::new("Open the sign-in page").size(12.0).color(p.accent)).sense(Sense::click())).clicked()
                            {
                                crate::paths::open_path(std::path::Path::new(link));
                            }
                            if ui.add(egui::Label::new(egui::RichText::new("Cancel").size(12.0).color(p.danger)).sense(Sense::click())).clicked() {
                                cancel_signin = true;
                            }
                        });
                    } else {
                        ui.label(
                            egui::RichText::new(if signed {
                                format!("✓ Signed in to {}", kind.label())
                            } else {
                                format!("A {} page opens in your browser; OTM never sees your password.", kind.label())
                            })
                            .size(12.0)
                            .color(if signed { p.success } else { p.muted }),
                        );
                        ui.add_space(4.0);
                        let label = if signed { "Sign in again".to_string() } else { format!("Sign in with {}", kind.label()) };
                        let ready = crate::uploads::engine_ready();
                        if ui.add_enabled_ui(ready, |ui| {
                            ui.set_width(ui.available_width());
                            if signed { widgets::secondary(ui, p, &label, None) } else { widgets::primary(ui, p, &label, None) }
                        }).inner.clicked() {
                            ed.result = None;
                            ed.signin = Some(SignIn::start(kind, ui.ctx().clone()));
                        }
                    }
                    if cancel_signin {
                        ed.signin = None;
                    }
                    ui.add_space(6.0);
                }
            }
            if changed {
                ed.result = None;
                ed.confirm_delete = false;
            }
            ui.add_space(6.0);
            ui.columns(2, |cols| {
                cols[0].set_width(cols[0].available_width());
                let testing = ed.testing.is_some();
                let label = if testing { "Testing…" } else { "Test connection" };
                if cols[0].add_enabled_ui(!testing, |ui| widgets::secondary(ui, p, label, None)).inner.clicked() {
                    ed.result = None;
                    ed.testing = Some(Uploads::test(ed.connector(), ed.secret_arg()));
                }
                if widgets::primary(&mut cols[1], p, "Save", None).clicked() {
                    let secret = if ed.secret.is_empty() && ed.secret_saved { None } else { Some(ed.secret.as_str()) };
                    match uploads.save_connector(ed.connector(), secret) {
                        Ok(id) => {
                            toast = Some("Destination saved".to_string());
                            if let Some(c) = uploads.connector(id) {
                                let saved = uploads.has_secret(&c);
                                let result = ed.result.take();
                                *ed = Editor::new(c, saved);
                                ed.result = result;
                            }
                        }
                        Err(e) => ed.result = Some(Err(e)),
                    }
                }
            });
            if ed.testing.is_some() {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new());
                    ui.label(egui::RichText::new("Connecting…").size(12.5).color(p.muted));
                });
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
            }
            if let Some(r) = &ed.result {
                ui.add_space(8.0);
                let (text, c) = match r {
                    Ok(t) => (format!("✓ {t}"), p.success),
                    Err(e) => (e.clone(), p.danger),
                };
                egui::Frame::new()
                    .fill(theme::tint(c, 0.10))
                    .stroke(Stroke::new(1.0, theme::tint(c, 0.40)))
                    .corner_radius(9)
                    .inner_margin(egui::Margin::symmetric(10, 8))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(egui::RichText::new(text).size(12.5).color(c));
                    });
            }
            if ed.form.id != 0 {
                ui.add_space(10.0);
                let label = if ed.confirm_delete { "Click again to remove this destination" } else { "Remove destination" };
                if ui.add(egui::Label::new(egui::RichText::new(label).size(12.0).color(p.danger)).sense(Sense::click())).clicked() {
                    if ed.confirm_delete {
                        uploads.remove_connector(ed.form.id);
                        toast = Some("Destination removed".into());
                        close = true;
                    } else {
                        ed.confirm_delete = true;
                    }
                }
            }
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(format!("The {} is kept in {}.", kind.secret_label().to_lowercase(), secrets::backend()))
                    .size(11.5)
                    .color(p.muted),
            );
        });
        if close {
            self.connect.editor = None;
        }
        if let Some(t) = toast {
            self.toast(t);
        }
    }
}

fn heading(ui: &mut egui::Ui, p: &Palette, title: &str, hint: &str) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(title)
                .font(theme::bold(11.0))
                .color(p.muted),
        );
        if !hint.is_empty() {
            ui.label(egui::RichText::new(hint).size(11.5).color(p.muted));
        }
    });
    ui.add_space(4.0);
}
