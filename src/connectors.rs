//! Upload destinations ("connectors"): SFTP, FTP, WebDAV and S3-compatible servers.
//!
//! Settings are saved in `connectors.json`; the password / secret key of each one is kept
//! in the system keychain (see `secrets`). rclone receives everything through environment
//! variables of a remote named `otm`, so nothing secret ever appears on a command line.

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    Sftp,
    Ftp,
    Webdav,
    S3,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Sftp, Kind::Ftp, Kind::Webdav, Kind::S3];

    pub fn label(self) -> &'static str {
        match self {
            Kind::Sftp => "SFTP",
            Kind::Ftp => "FTP / FTPS",
            Kind::Webdav => "WebDAV",
            Kind::S3 => "S3-compatible",
        }
    }
    pub fn mono(self) -> &'static str {
        match self {
            Kind::Sftp => "SF",
            Kind::Ftp => "FT",
            Kind::Webdav => "WD",
            Kind::S3 => "S3",
        }
    }
    pub fn blurb(self) -> &'static str {
        match self {
            Kind::Sftp => "Any SSH server. Key, agent or password.",
            Kind::Ftp => "Classic hosting and NAS boxes.",
            Kind::Webdav => "Nextcloud, ownCloud, NAS.",
            Kind::S3 => "AWS, Backblaze B2, R2, MinIO, Wasabi.",
        }
    }
    pub fn secret_label(self) -> &'static str {
        match self {
            Kind::S3 => "Secret key",
            Kind::Webdav => "Password or app password",
            _ => "Password",
        }
    }
    fn rclone_type(self) -> &'static str {
        match self {
            Kind::Sftp => "sftp",
            Kind::Ftp => "ftp",
            Kind::Webdav => "webdav",
            Kind::S3 => "s3",
        }
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
    pub user: String,
    /// Remote folder files are sent to. For S3: `bucket/optional/prefix`.
    pub folder: String,
    /// WebDAV address.
    pub url: String,
    /// WebDAV server flavour: nextcloud, owncloud or other.
    pub vendor: String,
    /// S3
    pub endpoint: String,
    pub region: String,
    pub access_key: String,
    /// FTP over TLS (explicit FTPS).
    pub tls: bool,
    pub auth: SftpAuth,
    pub key_file: String,
}

/// Name rclone knows our remote by (configured through `RCLONE_CONFIG_OTM_*`).
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
        }
        if self.secret_required() && !has_secret {
            return Err(format!("{} is required.", self.kind.secret_label()));
        }
        Ok(())
    }

    /// Settings for rclone's `otm` remote. `secret` is what the user typed, already
    /// obscured with `rclone obscure` for backends that expect it (`needs_obscured_secret`).
    pub fn rclone_env(&self, secret: Option<&str>) -> Vec<(String, String)> {
        let mut v: Vec<(&str, String)> = vec![("TYPE", self.kind.rclone_type().into())];
        let secret = secret.filter(|s| !s.is_empty());
        let port = self
            .port
            .or(self.kind.default_port())
            .map(|p| p.to_string());
        match self.kind {
            Kind::Sftp => {
                v.push(("HOST", self.host.trim().into()));
                v.push(("USER", self.user.trim().into()));
                if let Some(p) = port {
                    v.push(("PORT", p));
                }
                match self.auth {
                    SftpAuth::Password => {
                        if let Some(s) = secret {
                            v.push(("PASS", s.into()));
                        }
                    }
                    SftpAuth::KeyFile => {
                        v.push(("KEY_FILE", expand_home(self.key_file.trim())));
                        if let Some(s) = secret {
                            v.push(("KEY_FILE_PASS", s.into()));
                        }
                    }
                    // with no password and no key file rclone asks the ssh-agent
                    SftpAuth::Agent => {}
                }
            }
            Kind::Ftp => {
                v.push(("HOST", self.host.trim().into()));
                let user = self.user.trim();
                v.push((
                    "USER",
                    if user.is_empty() { "anonymous" } else { user }.into(),
                ));
                if let Some(p) = port {
                    v.push(("PORT", p));
                }
                if let Some(s) = secret {
                    v.push(("PASS", s.into()));
                }
                if self.tls {
                    v.push(("EXPLICIT_TLS", "true".into()));
                }
            }
            Kind::Webdav => {
                v.push(("URL", self.url.trim().trim_end_matches('/').into()));
                let vendor = match self.vendor.as_str() {
                    "nextcloud" | "owncloud" | "sharepoint" => self.vendor.clone(),
                    _ => "other".into(),
                };
                v.push(("VENDOR", vendor));
                if !self.user.trim().is_empty() {
                    v.push(("USER", self.user.trim().into()));
                }
                if let Some(s) = secret {
                    v.push(("PASS", s.into()));
                }
            }
            Kind::S3 => {
                let endpoint = self.endpoint.trim();
                v.push((
                    "PROVIDER",
                    if endpoint.is_empty() || endpoint.contains("amazonaws.com") {
                        "AWS"
                    } else {
                        "Other"
                    }
                    .into(),
                ));
                v.push(("ENV_AUTH", "false".into()));
                v.push(("ACCESS_KEY_ID", self.access_key.trim().into()));
                if let Some(s) = secret {
                    v.push(("SECRET_ACCESS_KEY", s.into()));
                }
                if !endpoint.is_empty() {
                    v.push(("ENDPOINT", endpoint.into()));
                }
                if !self.region.trim().is_empty() {
                    v.push(("REGION", self.region.trim().into()));
                }
                // buckets usually exist already and keys are often scoped to one bucket
                v.push(("NO_CHECK_BUCKET", "true".into()));
            }
        }
        v.into_iter()
            .map(|(k, val)| (format!("RCLONE_CONFIG_{}_{k}", REMOTE.to_uppercase()), val))
            .collect()
    }

    /// rclone stores most passwords "obscured"; S3 secret keys are passed as they are.
    pub fn needs_obscured_secret(&self) -> bool {
        self.kind != Kind::S3
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
