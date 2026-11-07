//! Upload destinations ("connectors"): servers (SFTP, FTP, WebDAV, S3-compatible) and
//! cloud drives (Google Drive, OneDrive, Dropbox, MEGA, pCloud, Box).
//!
//! Settings are saved in `connectors.json`; the password, secret key or sign-in token of
//! each one is kept in the system keychain (see `secrets`). rclone receives servers through
//! environment variables of a remote named `otm`; drives that sign in on the provider's
//! own page get a short-lived private config file instead, so rclone can write a renewed
//! token back (`rclone::Remote`). Nothing secret ever appears on a command line.

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    Sftp,
    Ftp,
    Webdav,
    S3,
    Gdrive,
    Onedrive,
    Dropbox,
    Mega,
    Pcloud,
    Box,
}

impl Kind {
    pub const SERVERS: [Kind; 4] = [Kind::Sftp, Kind::Ftp, Kind::Webdav, Kind::S3];
    pub const DRIVES: [Kind; 6] = [
        Kind::Gdrive,
        Kind::Onedrive,
        Kind::Dropbox,
        Kind::Mega,
        Kind::Pcloud,
        Kind::Box,
    ];

    /// Signs in on the provider's own web page (OAuth); the stored secret is the token.
    pub fn uses_browser_sign_in(self) -> bool {
        matches!(
            self,
            Kind::Gdrive | Kind::Onedrive | Kind::Dropbox | Kind::Pcloud | Kind::Box
        )
    }

    pub fn is_drive(self) -> bool {
        Kind::DRIVES.contains(&self)
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Sftp => "SFTP",
            Kind::Ftp => "FTP / FTPS",
            Kind::Webdav => "WebDAV",
            Kind::S3 => "S3-compatible",
            Kind::Gdrive => "Google Drive",
            Kind::Onedrive => "OneDrive",
            Kind::Dropbox => "Dropbox",
            Kind::Mega => "MEGA",
            Kind::Pcloud => "pCloud",
            Kind::Box => "Box",
        }
    }
    pub fn mono(self) -> &'static str {
        match self {
            Kind::Sftp => "SF",
            Kind::Ftp => "FT",
            Kind::Webdav => "WD",
            Kind::S3 => "S3",
            Kind::Gdrive => "GD",
            Kind::Onedrive => "OD",
            Kind::Dropbox => "DB",
            Kind::Mega => "MG",
            Kind::Pcloud => "PC",
            Kind::Box => "BX",
        }
    }
    pub fn blurb(self) -> &'static str {
        match self {
            Kind::Sftp => "Any SSH server. Key, agent or password.",
            Kind::Ftp => "Classic hosting and NAS boxes.",
            Kind::Webdav => "Nextcloud, ownCloud, NAS.",
            Kind::S3 => "AWS, Backblaze B2, R2, MinIO, Wasabi.",
            Kind::Gdrive => "Upload and create share links.",
            Kind::Onedrive => "Personal and work accounts.",
            Kind::Dropbox => "Upload and create share links.",
            Kind::Mega => "End-to-end encrypted storage.",
            Kind::Pcloud => "EU or US data region.",
            Kind::Box => "Work and personal accounts.",
        }
    }
    pub fn secret_label(self) -> &'static str {
        match self {
            Kind::S3 => "Secret key",
            Kind::Webdav => "Password or app password",
            k if k.uses_browser_sign_in() => "Sign-in token",
            _ => "Password",
        }
    }
    fn rclone_type(self) -> &'static str {
        match self {
            Kind::Sftp => "sftp",
            Kind::Ftp => "ftp",
            Kind::Webdav => "webdav",
            Kind::S3 => "s3",
            Kind::Gdrive => "drive",
            Kind::Onedrive => "onedrive",
            Kind::Dropbox => "dropbox",
            Kind::Mega => "mega",
            Kind::Pcloud => "pcloud",
            Kind::Box => "box",
        }
    }
    /// Backend name for `rclone authorize`.
    pub fn authorize_type(self) -> Option<&'static str> {
        self.uses_browser_sign_in().then(|| self.rclone_type())
    }
    pub fn default_port(self) -> Option<u16> {
        match self {
            Kind::Sftp => Some(22),
            Kind::Ftp => Some(21),
            _ => None,
        }
    }
}

/// How an SFTP connection signs in.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum SftpAuth {
    /// Keys loaded in the running ssh-agent (Pageant / OpenSSH agent).
    #[default]
    Agent,
    /// A private key file; the stored secret is its passphrase, if any.
    KeyFile,
    Password,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct Connector {
    pub id: u64,
    pub name: String,
    pub kind: Kind,
    /// SFTP / FTP
    pub host: String,
    pub port: Option<u16>,
    /// User name (servers) or account email (MEGA).
    pub user: String,
    /// Remote folder files are sent to. For S3: `bucket/optional/prefix`.
    pub folder: String,
    /// WebDAV address.
    pub url: String,
    /// WebDAV server flavour: nextcloud, owncloud or other.
    pub vendor: String,
    /// S3 endpoint.
    pub endpoint: String,
    /// S3 region; pCloud data region (`us` or `eu`).
    pub region: String,
    pub access_key: String,
    /// FTP over TLS (explicit FTPS).
    pub tls: bool,
    pub auth: SftpAuth,
    pub key_file: String,
    /// OneDrive: the drive files go to (found after sign-in).
    pub drive_id: String,
    pub drive_type: String,
}

/// Name rclone knows our remote by.
pub const REMOTE: &str = "otm";

impl Connector {
    pub fn new(kind: Kind) -> Self {
        Connector {
            kind,
            port: kind.default_port(),
            vendor: if kind == Kind::Webdav {
                "other".into()
            } else {
                String::new()
            },
            region: if kind == Kind::Pcloud {
                "us".into()
            } else {
                String::new()
            },
            folder: if kind.is_drive() {
                "OTM".into()
            } else {
                String::new()
            },
            ..Default::default()
        }
    }

    pub fn secret_key(&self) -> String {
        format!("connector-{}", self.id)
    }

    /// Whether this connection signs in with the stored secret (otherwise the secret is
    /// optional, e.g. an SFTP key without passphrase).
    pub fn secret_required(&self) -> bool {
        match self.kind {
            Kind::Sftp => self.auth == SftpAuth::Password,
            Kind::Ftp => !self.user.trim().is_empty() && self.user.trim() != "anonymous",
            Kind::Webdav => !self.user.trim().is_empty(),
            Kind::S3 => !self.access_key.trim().is_empty(),
            // MEGA needs its password, the others their sign-in token
            _ => true,
        }
    }

    pub fn display_name(&self) -> String {
        if self.name.trim().is_empty() {
            self.address()
        } else {
            self.name.trim().to_string()
        }
    }

    /// Short address shown under the name: `deploy@203.0.113.10:/var/www`.
    pub fn address(&self) -> String {
        let folder = self.folder.trim();
        match self.kind {
            Kind::Sftp | Kind::Ftp => {
                let user = self.user.trim();
                let at = if user.is_empty() {
                    String::new()
                } else {
                    format!("{user}@")
                };
                format!("{at}{}:{}", self.host.trim(), folder_or_home(folder))
            }
            Kind::Webdav => {
                let base = self.url.trim().trim_end_matches('/');
                if folder.is_empty() {
                    base.to_string()
                } else {
                    format!("{base}/{}", folder.trim_matches('/'))
                }
            }
            Kind::S3 => {
                let host = crate::model::host_of(self.endpoint.trim());
                let host = if host.is_empty() {
                    self.endpoint.trim().to_string()
                } else {
                    host
                };
                if host.is_empty() {
                    format!("s3://{}", folder.trim_matches('/'))
                } else {
                    format!("{host} · {}", folder.trim_matches('/'))
                }
            }
            k => {
                let account = self.user.trim();
                let place = if k == Kind::Mega && !account.is_empty() {
                    account
                } else {
                    k.label()
                };
                format!("{place} · /{}", folder.trim_matches('/'))
            }
        }
    }

    /// What is missing before the connection can be used.
    pub fn validate(&self, has_secret: bool) -> Result<(), String> {
        let need = |v: &str, what: &str| {
            if v.trim().is_empty() {
                Err(format!("{what} is required."))
            } else {
                Ok(())
            }
        };
        match self.kind {
            Kind::Sftp | Kind::Ftp => {
                need(&self.host, "Host")?;
                if self.host.trim().contains(['/', ' ']) {
                    return Err("Host is a name or address only, like sftp.example.com.".into());
                }
                if self.kind == Kind::Sftp {
                    need(&self.user, "User")?;
                    if self.auth == SftpAuth::KeyFile {
                        need(&self.key_file, "Key file")?;
                    }
                }
            }
            Kind::Webdav => {
                need(&self.url, "Address")?;
                let u = self.url.trim();
                if !(u.starts_with("https://") || u.starts_with("http://")) {
                    return Err("The address starts with https://".into());
                }
            }
            Kind::S3 => {
                need(&self.folder, "Bucket")?;
                need(&self.access_key, "Access key")?;
            }
            Kind::Mega => need(&self.user, "Email")?,
            _ => {}
        }
        if self.secret_required() && !has_secret {
            if self.kind.uses_browser_sign_in() {
                return Err(format!("Sign in to {} first.", self.kind.label()));
            }
            return Err(format!("{} is required.", self.kind.secret_label()));
        }
        Ok(())
    }

    /// Settings for rclone's `otm` remote as environment variables (servers, MEGA).
    pub fn rclone_env(&self, secret: Option<&str>) -> Vec<(String, String)> {
        self.rclone_options(secret)
            .into_iter()
            .map(|(k, val)| {
                (
                    format!(
                        "RCLONE_CONFIG_{}_{}",
                        REMOTE.to_uppercase(),
                        k.to_uppercase()
                    ),
                    val,
                )
            })
            .collect()
    }

    /// The same settings as an rclone config file (drives with sign-in tokens, so
    /// rclone can write a renewed token back).
    pub fn rclone_ini(&self, secret: Option<&str>) -> String {
        let mut s = format!("[{REMOTE}]\n");
        for (k, v) in self.rclone_options(secret) {
            // values stay on one line: never let one start another setting or section
            let v = v.replace(['\r', '\n'], "");
            s += &format!("{k} = {v}\n");
        }
        s
    }

    /// Settings for rclone's `otm` remote. `secret` is the password (already obscured
    /// with `rclone obscure` where `needs_obscured_secret`), S3 secret key or sign-in token.
    fn rclone_options(&self, secret: Option<&str>) -> Vec<(&'static str, String)> {
        let mut v: Vec<(&'static str, String)> = vec![("type", self.kind.rclone_type().into())];
        let secret = secret.filter(|s| !s.is_empty());
        let port = self
            .port
            .or(self.kind.default_port())
            .map(|p| p.to_string());
        match self.kind {
            Kind::Sftp => {
                v.push(("host", self.host.trim().into()));
                v.push(("user", self.user.trim().into()));
                if let Some(p) = port {
                    v.push(("port", p));
                }
                match self.auth {
                    SftpAuth::Password => {
                        if let Some(s) = secret {
                            v.push(("pass", s.into()));
                        }
                    }
                    SftpAuth::KeyFile => {
                        v.push(("key_file", expand_home(self.key_file.trim())));
                        if let Some(s) = secret {
                            v.push(("key_file_pass", s.into()));
                        }
                    }
                    // with no password and no key file rclone asks the ssh-agent
                    SftpAuth::Agent => {}
                }
            }
            Kind::Ftp => {
                v.push(("host", self.host.trim().into()));
                let user = self.user.trim();
                v.push((
                    "user",
                    if user.is_empty() { "anonymous" } else { user }.into(),
                ));
                if let Some(p) = port {
                    v.push(("port", p));
                }
                if let Some(s) = secret {
                    v.push(("pass", s.into()));
                }
                if self.tls {
                    v.push(("explicit_tls", "true".into()));
                }
            }
            Kind::Webdav => {
                v.push(("url", self.url.trim().trim_end_matches('/').into()));
                let vendor = match self.vendor.as_str() {
                    "nextcloud" | "owncloud" | "sharepoint" => self.vendor.clone(),
                    _ => "other".into(),
                };
                v.push(("vendor", vendor));
                if !self.user.trim().is_empty() {
                    v.push(("user", self.user.trim().into()));
                }
                if let Some(s) = secret {
                    v.push(("pass", s.into()));
                }
            }
            Kind::S3 => {
                let endpoint = self.endpoint.trim();
                v.push((
                    "provider",
                    if endpoint.is_empty() || endpoint.contains("amazonaws.com") {
                        "AWS"
                    } else {
                        "Other"
                    }
                    .into(),
                ));
                v.push(("env_auth", "false".into()));
                v.push(("access_key_id", self.access_key.trim().into()));
                if let Some(s) = secret {
                    v.push(("secret_access_key", s.into()));
                }
                if !endpoint.is_empty() {
                    v.push(("endpoint", endpoint.into()));
                }
                if !self.region.trim().is_empty() {
                    v.push(("region", self.region.trim().into()));
                }
                // buckets usually exist already and keys are often scoped to one bucket
                v.push(("no_check_bucket", "true".into()));
            }
            Kind::Mega => {
                v.push(("user", self.user.trim().into()));
                if let Some(s) = secret {
                    v.push(("pass", s.into()));
                }
            }
            Kind::Gdrive | Kind::Onedrive | Kind::Dropbox | Kind::Pcloud | Kind::Box => {
                if let Some(s) = secret {
                    v.push(("token", s.into()));
                }
                match self.kind {
                    Kind::Gdrive => v.push(("scope", "drive".into())),
                    Kind::Onedrive if !self.drive_id.is_empty() => {
                        v.push(("drive_id", self.drive_id.clone()));
                        let t = if self.drive_type.is_empty() {
                            "personal"
                        } else {
                            &self.drive_type
                        };
                        v.push(("drive_type", t.into()));
                    }
                    Kind::Pcloud => {
                        let host = if self.region == "eu" {
                            "eapi.pcloud.com"
                        } else {
                            "api.pcloud.com"
                        };
                        v.push(("hostname", host.into()));
                    }
                    _ => {}
                }
            }
        }
        v
    }

    /// rclone stores passwords "obscured"; S3 secret keys and sign-in tokens are passed
    /// as they are.
    pub fn needs_obscured_secret(&self) -> bool {
        self.kind != Kind::S3 && !self.kind.uses_browser_sign_in()
    }

    /// `otm:folder` (the destination folder on the remote).
    pub fn remote_folder(&self) -> String {
        format!("{REMOTE}:{}", self.folder.trim().trim_end_matches('/'))
    }

    /// `otm:folder/name`
    pub fn remote_path(&self, name: &str) -> String {
        let folder = self.folder.trim().trim_end_matches('/');
        if folder.is_empty() {
            format!("{REMOTE}:{name}")
        } else {
            format!("{REMOTE}:{folder}/{name}")
        }
    }
}

fn folder_or_home(folder: &str) -> &str {
    if folder.is_empty() { "~" } else { folder }
}

fn expand_home(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/").or_else(|| p.strip_prefix("~\\")) {
        crate::paths::home().join(rest).display().to_string()
    } else {
        p.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of(c: &Connector, s: Option<&str>) -> std::collections::BTreeMap<String, String> {
        c.rclone_env(s)
            .into_iter()
            .map(|(k, v)| (k.trim_start_matches("RCLONE_CONFIG_OTM_").to_string(), v))
            .collect()
    }

    #[test]
    fn sftp_settings_for_rclone() {
        let mut c = Connector::new(Kind::Sftp);
        c.host = " 203.0.113.10 ".into();
        c.user = "deploy".into();
        c.folder = "/var/www/releases/".into();
        let e = env_of(&c, None);
        assert_eq!(e["TYPE"], "sftp");
        assert_eq!(e["HOST"], "203.0.113.10");
        assert_eq!(e["PORT"], "22");
        assert!(!e.contains_key("KEY_FILE"));
        assert!(!e.contains_key("PASS"));
        c.auth = SftpAuth::Password;
        assert_eq!(env_of(&c, Some("obscured"))["PASS"], "obscured");
        assert_eq!(c.remote_path("a b.zip"), "otm:/var/www/releases/a b.zip");
        assert_eq!(c.address(), "deploy@203.0.113.10:/var/www/releases/");
        assert!(
            c.rclone_env(None)
                .iter()
                .all(|(k, _)| k.starts_with("RCLONE_CONFIG_OTM_"))
        );
    }

    #[test]
    fn s3_secret_is_not_obscured_and_bucket_is_the_folder() {
        let mut c = Connector::new(Kind::S3);
        c.endpoint = "https://s3.eu-central-003.backblazeb2.com".into();
        c.access_key = "K003".into();
        c.folder = "otm-releases/builds".into();
        assert!(!c.needs_obscured_secret());
        let e = env_of(&c, Some("plain-secret"));
        assert_eq!(e["PROVIDER"], "Other");
        assert_eq!(e["SECRET_ACCESS_KEY"], "plain-secret");
        assert_eq!(c.remote_path("x.zip"), "otm:otm-releases/builds/x.zip");
        assert_eq!(
            c.address(),
            "s3.eu-central-003.backblazeb2.com · otm-releases/builds"
        );
    }

    #[test]
    fn drives_get_a_config_file_with_their_token() {
        let mut c = Connector::new(Kind::Onedrive);
        assert_eq!(c.folder, "OTM", "drives send to an OTM folder by default");
        assert_eq!(c.validate(false).unwrap_err(), "Sign in to OneDrive first.");
        assert!(c.validate(true).is_ok());
        assert!(!c.needs_obscured_secret());
        c.drive_id = "b!abc".into();
        c.drive_type = "business".into();
        let ini = c.rclone_ini(Some("{\"access_token\":\"x\"}\n[evil]\ntype = local"));
        assert!(ini.starts_with("[otm]\ntype = onedrive\n"));
        assert!(ini.contains("token = {\"access_token\":\"x\"}[evil]type = local\n"));
        assert_eq!(ini.matches('[').count(), 2, "no injected section: {ini}");
        assert!(ini.contains("drive_id = b!abc\ndrive_type = business\n"));
        assert_eq!(c.address(), "OneDrive · /OTM");

        let mut p = Connector::new(Kind::Pcloud);
        p.region = "eu".into();
        assert!(
            p.rclone_ini(Some("t"))
                .contains("hostname = eapi.pcloud.com")
        );
        assert_eq!(Kind::Gdrive.authorize_type(), Some("drive"));
        assert_eq!(Kind::Mega.authorize_type(), None);
    }

    #[test]
    fn mega_signs_in_with_email_and_password() {
        let mut m = Connector::new(Kind::Mega);
        assert_eq!(m.validate(true).unwrap_err(), "Email is required.");
        m.user = "me@example.com".into();
        assert_eq!(m.validate(false).unwrap_err(), "Password is required.");
        assert!(m.needs_obscured_secret());
        let e = env_of(&m, Some("obscured"));
        assert_eq!(e["TYPE"], "mega");
        assert_eq!(e["USER"], "me@example.com");
        assert_eq!(m.address(), "me@example.com · /OTM");
    }

    #[test]
    fn validation_names_what_is_missing() {
        let mut c = Connector::new(Kind::Sftp);
        assert_eq!(c.validate(false).unwrap_err(), "Host is required.");
        c.host = "example.com".into();
        c.user = "me".into();
        assert!(c.validate(false).is_ok(), "agent sign-in needs no secret");
        c.auth = SftpAuth::Password;
        assert_eq!(c.validate(false).unwrap_err(), "Password is required.");
        assert!(c.validate(true).is_ok());
        c.host = "sftp://example.com".into();
        assert!(c.validate(true).is_err());

        let mut w = Connector::new(Kind::Webdav);
        w.url = "cloud.example.com".into();
        assert!(w.validate(false).unwrap_err().contains("https://"));
        w.url = "https://cloud.example.com/remote.php/dav/files/me".into();
        assert!(w.validate(false).is_ok(), "anonymous WebDAV is allowed");
    }

    #[test]
    fn old_files_load_with_defaults() {
        let c: Connector = serde_json::from_str(r#"{"id":3,"kind":"ftp","host":"h"}"#).unwrap();
        assert_eq!(c.kind, Kind::Ftp);
        assert_eq!(c.auth, SftpAuth::Agent);
        assert!(!c.tls);
    }
}
