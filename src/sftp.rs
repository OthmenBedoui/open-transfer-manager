//! Sends files to a server over SFTP.
//!
//! Pieces can arrive in any order, so each one is written at its offset in a temporary file next
//! to the final name. Only when everything is there does the file get its name, in one step.
//! SFTP has no standard checksum, so by default each piece is read back and compared.
//! A server key that isn't already trusted is refused: nothing is sent to an unverified server.
use crate::destination::{Destination, SendError};
use crate::s3::sha256_hex;
use russh::client::{self, Handle};
use russh::keys::{self, HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::{OpenFlags, StatusCode};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

const PART_SIZE: u64 = 8 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone, Debug)]
pub enum HostKeys {
    /// Trust only what this `known_hosts` file lists.
    KnownHosts(PathBuf),
    /// Trust only the key with this `SHA256:...` fingerprint.
    Pinned(String),
    /// Trust a first-seen key and remember it there; a changed key is still refused.
    AcceptNew(PathBuf),
}

#[derive(Clone)]
pub enum Login {
    Password(String),
    KeyFile { path: PathBuf, passphrase: Option<String> },
}

impl std::fmt::Debug for Login {
    // Never prints a secret, even in an error report.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Login::Password(_) => write!(f, "Password(hidden)"),
            Login::KeyFile { path, .. } => write!(f, "KeyFile({})", path.display()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Target {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub path: String,
    pub login: Login,
    pub host_keys: HostKeys,
    /// Read each piece back and compare it. Slower, but SFTP offers no other proof.
    pub verify: bool,
    pub overwrite: bool,
}

/// Reads `sftp://user@host:port/some/path`.
pub fn parse_address(address: &str) -> Option<(String, u16, String, String)> {
    let u = url::Url::parse(address).ok()?;
    if u.scheme() != "sftp" || u.username().is_empty() || u.path().len() < 2 {
        return None;
    }
    let path = url::form_urlencoded::parse(u.path().replace('+', "%2B").as_bytes()).next().map(|(k, _)| k.to_string())?;
    Some((u.host_str()?.to_string(), u.port().unwrap_or(22), u.username().to_string(), path))
}

struct Checker {
    host: String,
    port: u16,
    policy: HostKeys,
    problem: Arc<Mutex<Option<String>>>,
}

impl client::Handler for Checker {
    type Error = russh::Error;

    async fn check_server_key(&mut self, offered: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        let PublicKeyOrCertificate::PublicKey { key, .. } = offered else {
            *self.problem.lock().unwrap() = Some("the server offered a certificate, which isn't supported".into());
            return Ok(false);
        };
        let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
        let refuse = |why: String| {
            *self.problem.lock().unwrap() = Some(why);
            Ok(false)
        };
        let unknown = format!(
            "the server {}:{} is not trusted yet. Its key fingerprint is {}. Compare it with the one your host gives you, then pass --host-key {}",
            self.host, self.port, fingerprint, fingerprint
        );
        match &self.policy {
            HostKeys::Pinned(wanted) => {
                if *wanted == fingerprint {
                    Ok(true)
                } else {
                    refuse(format!("the server's key is {}, not the expected {}", fingerprint, wanted))
                }
            }
            HostKeys::KnownHosts(file) | HostKeys::AcceptNew(file) => match keys::check_known_hosts_path(&self.host, self.port, key, file) {
                Ok(true) => Ok(true),
                Err(keys::Error::KeyChanged { line }) => refuse(format!(
                    "the server's key CHANGED (it is now {}; the old one is at line {} of {}). This can mean someone is intercepting the connection. Nothing was sent.",
                    fingerprint,
                    line,
                    file.display()
                )),
                _ => {
                    if let HostKeys::AcceptNew(_) = &self.policy {
                        match keys::known_hosts::learn_known_hosts_path(&self.host, self.port, key, file) {
                            Ok(()) => Ok(true),
                            Err(e) => refuse(format!("cannot remember the server's key: {}", e)),
                        }
                    } else {
                        refuse(unknown)
                    }
                }
            },
        }
    }
}

struct Connection {
    // Keeps the SSH connection alive for as long as its SFTP session.
    _ssh: Handle<Checker>,
    sftp: SftpSession,
}

fn classify(context: &str, e: SftpError) -> SendError {
    let text = format!("{}: {}", context, e);
    match &e {
        SftpError::Status(s) if s.status_code == StatusCode::PermissionDenied => SendError::Fatal(text),
        _ => SendError::Retry(text, None),
    }
}

pub struct SftpDestination {
    target: Target,
    runtime: tokio::runtime::Runtime,
    idle: Mutex<Vec<Connection>>,
}

impl SftpDestination {
    pub fn new(target: Target) -> Result<Self, SendError> {
        let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().map_err(|e| SendError::Fatal(e.to_string()))?;
        Ok(SftpDestination { target, runtime, idle: Mutex::new(vec![]) })
    }

    async fn connect(&self) -> Result<Connection, SendError> {
        let t = &self.target;
        let problem = Arc::new(Mutex::new(None));
        let checker = Checker { host: t.host.clone(), port: t.port, policy: t.host_keys.clone(), problem: problem.clone() };
        let config = Arc::new(client::Config { inactivity_timeout: Some(TIMEOUT), ..Default::default() });
        let mut ssh = match tokio::time::timeout(TIMEOUT, client::connect(config, (t.host.as_str(), t.port), checker)).await {
            Ok(Ok(h)) => h,
            Ok(Err(e)) => {
                return Err(match problem.lock().unwrap().take() {
                    Some(why) => SendError::Fatal(why),
                    None => SendError::Retry(format!("cannot connect: {}", e), None),
                })
            }
            Err(_) => return Err(SendError::Retry("connecting took too long".into(), None)),
        };
        let accepted = match &t.login {
            Login::Password(p) => ssh.authenticate_password(&t.user, p).await,
            Login::KeyFile { path, passphrase } => {
                let key = keys::load_secret_key(path, passphrase.as_deref()).map_err(|e| SendError::Fatal(format!("cannot read the key {}: {}", path.display(), e)))?;
                let hash = ssh.best_supported_rsa_hash().await.map_err(|e| SendError::Retry(e.to_string(), None))?.flatten();
                ssh.authenticate_publickey(&t.user, PrivateKeyWithHashAlg::new(Arc::new(key), hash)).await
            }
        }
        .map_err(|e| SendError::Retry(format!("login failed: {}", e), None))?;
        if !accepted.success() {
            return Err(SendError::Fatal(format!("the server refused the login for {}", t.user)));
        }
        let channel = ssh.channel_open_session().await.map_err(|e| SendError::Retry(e.to_string(), None))?;
        channel.request_subsystem(true, "sftp").await.map_err(|e| SendError::Retry(e.to_string(), None))?;
        let sftp = SftpSession::new(channel.into_stream()).await.map_err(|e| SendError::Retry(format!("SFTP is not available: {}", e), None))?;
        Ok(Connection { _ssh: ssh, sftp })
    }

    /// Runs `work` on a connection, which goes back to the pool only if it worked.
    fn with_connection<T>(&self, work: impl AsyncFnOnce(&SftpSession) -> Result<T, SendError>) -> Result<T, SendError> {
        self.runtime.block_on(async {
            let existing = self.idle.lock().unwrap().pop();
            let connection = match existing {
                Some(c) => c,
                None => self.connect().await?,
            };
            let result = match tokio::time::timeout(TIMEOUT * 5, work(&connection.sftp)).await {
                Ok(r) => r,
                Err(_) => Err(SendError::Retry("the server took too long".into(), None)),
            };
            if result.is_ok() {
                self.idle.lock().unwrap().push(connection);
            }
            result
        })
    }

    fn temp_path(&self) -> String {
        format!("{}.otmpart", self.target.path)
    }
}

impl Destination for SftpDestination {
    fn describe(&self) -> String {
        format!("sftp://{}@{}:{}{}", self.target.user, self.target.host, self.target.port, self.target.path)
    }

    fn part_size(&self, _size: u64) -> u64 {
        PART_SIZE
    }

    fn start(&self, _size: u64) -> Result<String, SendError> {
        let (path, temp, overwrite) = (self.target.path.clone(), self.temp_path(), self.target.overwrite);
        self.with_connection(async move |sftp: &SftpSession| {
            if !overwrite && sftp.try_exists(path.clone()).await.map_err(|e| classify("checking the destination", e))? {
                return Err(SendError::Fatal(format!("{} already exists on the server. Pass --overwrite to replace it.", path)));
            }
            // Folders that already exist answer with an error, which is fine here.
            let mut so_far = String::new();
            let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
            for folder in &parts[..parts.len().saturating_sub(1)] {
                so_far.push('/');
                so_far.push_str(folder);
                let _ = sftp.create_dir(so_far.clone()).await;
            }
            let file = sftp.open_with_flags(temp.clone(), OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::TRUNCATE).await.map_err(|e| classify("creating the file", e))?;
            file.close().await.map_err(|e| SendError::Retry(e.to_string(), None))?;
            Ok(temp)
        })
    }

    fn still_valid(&self, id: &str) -> bool {
        let id = id.to_string();
        self.with_connection(async move |sftp: &SftpSession| sftp.try_exists(id).await.map_err(|e| classify("checking", e))).unwrap_or(false)
    }

    fn send_part(&self, id: &str, _number: u32, offset: u64, data: &[u8]) -> Result<String, SendError> {
        let id = id.to_string();
        let verify = self.target.verify;
        self.with_connection(async move |sftp: &SftpSession| {
            let mut file = sftp.open_with_flags(id.clone(), OpenFlags::WRITE).await.map_err(|e| classify("opening the file", e))?;
            let retry = |e: std::io::Error| SendError::Retry(format!("sending failed: {}", e), None);
            file.seek(std::io::SeekFrom::Start(offset)).await.map_err(retry)?;
            file.write_all(data).await.map_err(retry)?;
            file.flush().await.map_err(retry)?;
            file.close().await.map_err(retry)?;
            let sent = sha256_hex(data);
            if verify {
                let mut back = sftp.open_with_flags(id, OpenFlags::READ).await.map_err(|e| classify("reading back", e))?;
                back.seek(std::io::SeekFrom::Start(offset)).await.map_err(retry)?;
                let mut stored = vec![0u8; data.len()];
                back.read_exact(&mut stored).await.map_err(|e| SendError::Retry(format!("reading back failed: {}", e), None))?;
                back.close().await.map_err(retry)?;
                if sha256_hex(&stored) != sent {
                    return Err(SendError::Retry("the server stored different bytes than were sent".into(), None));
                }
            }
            Ok(sent)
        })
    }

    fn finish(&self, id: &str, _parts: &[(u32, String)], _size: u64) -> Result<(), SendError> {
        let (id, path, overwrite) = (id.to_string(), self.target.path.clone(), self.target.overwrite);
        self.with_connection(async move |sftp: &SftpSession| {
            if overwrite && sftp.try_exists(path.clone()).await.map_err(|e| classify("checking", e))? {
                sftp.remove_file(path.clone()).await.map_err(|e| classify("replacing the old file", e))?;
            }
            sftp.rename(id, path).await.map_err(|e| classify("naming the file", e))
        })
    }

    fn cancel(&self, id: &str) {
        let id = id.to_string();
        let _ = self.with_connection(async move |sftp: &SftpSession| sftp.remove_file(id).await.map_err(|e| classify("cleaning up", e)));
    }

    fn store_empty(&self) -> Result<(), SendError> {
        let temp = self.start(0)?;
        self.finish(&temp, &[], 0)
    }

    fn stored_size(&self) -> Result<u64, SendError> {
        let path = self.target.path.clone();
        self.with_connection(async move |sftp: &SftpSession| {
            let m = sftp.metadata(path).await.map_err(|e| classify("checking the stored file", e))?;
            Ok(m.len())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_are_read() {
        let a = parse_address("sftp://me@example.org:2222/data/my%20file.bin").unwrap();
        assert_eq!(a, ("example.org".to_string(), 2222, "me".to_string(), "/data/my file.bin".to_string()));
        assert_eq!(parse_address("sftp://me@example.org/a+b").unwrap().3, "/a+b");
        assert_eq!(parse_address("sftp://me@example.org/a").unwrap().1, 22);
        assert!(parse_address("sftp://example.org/a").is_none());
        assert!(parse_address("sftp://me@example.org/").is_none());
        assert!(parse_address("https://me@example.org/a").is_none());
    }

    #[test]
    fn secrets_never_print() {
        assert!(!format!("{:?}", Login::Password("hunter2".into())).contains("hunter2"));
    }
}
