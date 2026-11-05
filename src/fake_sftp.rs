//! An SFTP server for tests that misbehaves on demand. It speaks real SSH and real SFTP, and
//! keeps files in memory. It is not a full server, and passing against it isn't proof about others.
use russh::keys::{Algorithm, HashAlg, PrivateKey, PublicKey};
use russh::server::{Auth, ChannelOpenHandle, Msg, PublicKeyOffer, Session, VerifiedPublicKey};
use russh::{Channel, ChannelId};
use russh_sftp::protocol::{Attrs, Data, FileAttributes, Handle, Name, OpenFlags, Status, StatusCode, Version};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Notify;

#[derive(Clone, Debug, Default)]
pub struct Faults {
    /// Every Nth write is refused with an error.
    pub error_every: u64,
    /// Every Nth write cuts the whole connection, as a network failure would.
    pub cut_every: u64,
    /// The Nth write is silently stored with one wrong byte, once.
    pub corrupt_write: u64,
    /// Every write waits this long, to make the destination slow.
    pub write_delay_ms: u64,
}

#[derive(Default)]
pub struct Files {
    pub files: HashMap<String, Vec<u8>>,
    pub dirs: HashSet<String>,
}

#[derive(Default)]
pub struct Counters {
    pub connections: AtomicU64,
    pub writes: AtomicU64,
    pub bytes_written: AtomicU64,
    pub reads: AtomicU64,
    pub renames: AtomicU64,
    pub removes: AtomicU64,
}

pub struct FakeSftp {
    pub port: u16,
    pub files: Arc<Mutex<Files>>,
    pub counters: Arc<Counters>,
    pub fingerprint: String,
    pub public_key_line: String,
    _runtime: tokio::runtime::Runtime,
}

struct Shared {
    faults: Faults,
    files: Arc<Mutex<Files>>,
    counters: Arc<Counters>,
    password: String,
    allowed_key: Option<PublicKey>,
}

/// `password` opens the door; `allowed_key` too, if given. `/readonly` refuses all writes.
pub fn start(faults: Faults, password: &str, allowed_key: Option<PublicKey>) -> FakeSftp {
    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().expect("runtime");
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).expect("key");
    let fingerprint = key.public_key().fingerprint(HashAlg::Sha256).to_string();
    let public_key_line = key.public_key().to_openssh().expect("public key");
    let files = Arc::new(Mutex::new(Files::default()));
    files.lock().unwrap().dirs.extend(["/".to_string(), "/readonly".to_string()]);
    let counters = Arc::new(Counters::default());
    let shared = Arc::new(Shared { faults, files: files.clone(), counters: counters.clone(), password: password.to_string(), allowed_key });
    let config = Arc::new(russh::server::Config { keys: vec![key], auth_rejection_time: Duration::from_millis(10), auth_rejection_time_initial: Some(Duration::ZERO), ..Default::default() });
    let listener = runtime.block_on(tokio::net::TcpListener::bind(("127.0.0.1", 0))).expect("bind");
    let port = listener.local_addr().expect("address").port();
    runtime.spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            shared.counters.connections.fetch_add(1, Ordering::SeqCst);
            let (sh, config) = (shared.clone(), config.clone());
            tokio::spawn(async move {
                let cut = Arc::new(Notify::new());
                let handler = Ssh { shared: sh, channels: HashMap::new(), cut: cut.clone() };
                let Ok(running) = russh::server::run_stream(config, socket, handler).await else { return };
                // Dropping the session closes the socket without any goodbye.
                tokio::select! {
                    _ = running => {}
                    _ = cut.notified() => {}
                }
            });
        }
    });
    FakeSftp { port, files, counters, fingerprint, public_key_line, _runtime: runtime }
}

struct Ssh {
    shared: Arc<Shared>,
    channels: HashMap<ChannelId, Channel<Msg>>,
    cut: Arc<Notify>,
}

impl russh::server::Handler for Ssh {
    type Error = russh::Error;

    async fn auth_password(&mut self, _user: &str, password: &str) -> Result<Auth, Self::Error> {
        Ok(if password == self.shared.password { Auth::Accept } else { Auth::reject() })
    }

    async fn auth_publickey_offered(&mut self, _user: &str, offer: PublicKeyOffer<'_>) -> Result<Auth, Self::Error> {
        Ok(if self.shared.allowed_key.as_ref() == Some(offer.public_key) { Auth::Accept } else { Auth::reject() })
    }

    async fn auth_publickey(&mut self, _user: &str, key: VerifiedPublicKey<'_>) -> Result<Auth, Self::Error> {
        Ok(if self.shared.allowed_key.as_ref() == Some(key.public_key) { Auth::Accept } else { Auth::reject() })
    }

    async fn channel_open_session(&mut self, channel: Channel<Msg>, reply: ChannelOpenHandle, _session: &mut Session) -> Result<(), Self::Error> {
        self.channels.insert(channel.id(), channel);
        reply.accept().await;
        Ok(())
    }

    async fn channel_eof(&mut self, channel: ChannelId, session: &mut Session) -> Result<(), Self::Error> {
        session.close(channel)?;
        Ok(())
    }

    async fn subsystem_request(&mut self, id: ChannelId, name: &str, session: &mut Session) -> Result<(), Self::Error> {
        match (name, self.channels.remove(&id)) {
            ("sftp", Some(channel)) => {
                session.channel_success(id)?;
                let sftp = Sftp { shared: self.shared.clone(), cut: self.cut.clone(), handles: HashMap::new(), next: 0 };
                russh_sftp::server::run(channel.into_stream(), sftp).await;
            }
            _ => session.channel_failure(id)?,
        }
        Ok(())
    }
}

struct Sftp {
    shared: Arc<Shared>,
    cut: Arc<Notify>,
    handles: HashMap<String, String>,
    next: u64,
}

fn ok(id: u32) -> Status {
    Status { id, status_code: StatusCode::Ok, error_message: "Ok".into(), language_tag: "en-US".into() }
}

fn parent(path: &str) -> String {
    match path.rsplit_once('/') {
        Some(("", _)) | None => "/".to_string(),
        Some((p, _)) => p.to_string(),
    }
}

impl Sftp {
    fn lock(&self) -> std::sync::MutexGuard<'_, Files> {
        self.shared.files.lock().unwrap()
    }
}

impl russh_sftp::server::Handler for Sftp {
    type Error = StatusCode;

    fn unimplemented(&self) -> Self::Error {
        StatusCode::OpUnsupported
    }

    async fn init(&mut self, _version: u32, _extensions: HashMap<String, String>) -> Result<Version, Self::Error> {
        Ok(Version::new())
    }

    async fn open(&mut self, id: u32, filename: String, flags: OpenFlags, _attrs: FileAttributes) -> Result<Handle, Self::Error> {
        let wants_write = flags.intersects(OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::TRUNCATE);
        if wants_write && filename.starts_with("/readonly/") {
            return Err(StatusCode::PermissionDenied);
        }
        {
            let mut st = self.lock();
            if !st.dirs.contains(&parent(&filename)) {
                return Err(StatusCode::NoSuchFile);
            }
            if !st.files.contains_key(&filename) {
                if !flags.contains(OpenFlags::CREATE) {
                    return Err(StatusCode::NoSuchFile);
                }
                st.files.insert(filename.clone(), vec![]);
            } else if flags.contains(OpenFlags::TRUNCATE) {
                st.files.insert(filename.clone(), vec![]);
            }
        }
        self.next += 1;
        let handle = format!("h{}", self.next);
        self.handles.insert(handle.clone(), filename);
        Ok(Handle { id, handle })
    }

    async fn close(&mut self, id: u32, handle: String) -> Result<Status, Self::Error> {
        self.handles.remove(&handle);
        Ok(ok(id))
    }

    async fn write(&mut self, id: u32, handle: String, offset: u64, mut data: Vec<u8>) -> Result<Status, Self::Error> {
        let path = self.handles.get(&handle).cloned().ok_or(StatusCode::Failure)?;
        let n = self.shared.counters.writes.fetch_add(1, Ordering::SeqCst) + 1;
        let f = &self.shared.faults;
        if f.write_delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(f.write_delay_ms)).await;
        }
        if f.cut_every > 0 && n % f.cut_every == 0 {
            self.cut.notify_one();
            return Err(StatusCode::ConnectionLost);
        }
        if f.error_every > 0 && n % f.error_every == 0 {
            return Err(StatusCode::Failure);
        }
        if f.corrupt_write == n && !data.is_empty() {
            data[0] ^= 0xFF;
        }
        self.shared.counters.bytes_written.fetch_add(data.len() as u64, Ordering::SeqCst);
        let mut st = self.lock();
        let file = st.files.get_mut(&path).ok_or(StatusCode::NoSuchFile)?;
        let end = offset as usize + data.len();
        if file.len() < end {
            file.resize(end, 0);
        }
        file[offset as usize..end].copy_from_slice(&data);
        Ok(ok(id))
    }

    async fn read(&mut self, id: u32, handle: String, offset: u64, len: u32) -> Result<Data, Self::Error> {
        let path = self.handles.get(&handle).cloned().ok_or(StatusCode::Failure)?;
        self.shared.counters.reads.fetch_add(1, Ordering::SeqCst);
        let st = self.lock();
        let file = st.files.get(&path).ok_or(StatusCode::NoSuchFile)?;
        if offset as usize >= file.len() {
            return Err(StatusCode::Eof);
        }
        let end = (offset as usize + len as usize).min(file.len());
        Ok(Data { id, data: file[offset as usize..end].to_vec() })
    }

    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        let st = self.lock();
        let mut attrs = FileAttributes::default();
        if let Some(f) = st.files.get(&path) {
            attrs.size = Some(f.len() as u64);
            attrs.permissions = Some(0o100644);
        } else if st.dirs.contains(&path) {
            attrs.size = Some(0);
            attrs.permissions = Some(0o040755);
        } else {
            return Err(StatusCode::NoSuchFile);
        }
        Ok(Attrs { id, attrs })
    }

    async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        self.stat(id, path).await
    }

    async fn fstat(&mut self, id: u32, handle: String) -> Result<Attrs, Self::Error> {
        let path = self.handles.get(&handle).cloned().ok_or(StatusCode::Failure)?;
        self.stat(id, path).await
    }

    async fn mkdir(&mut self, id: u32, path: String, _attrs: FileAttributes) -> Result<Status, Self::Error> {
        let mut st = self.lock();
        if st.dirs.contains(&path) || !st.dirs.contains(&parent(&path)) {
            return Err(StatusCode::Failure);
        }
        st.dirs.insert(path);
        Ok(ok(id))
    }

    async fn remove(&mut self, id: u32, filename: String) -> Result<Status, Self::Error> {
        self.shared.counters.removes.fetch_add(1, Ordering::SeqCst);
        self.lock().files.remove(&filename).ok_or(StatusCode::NoSuchFile)?;
        Ok(ok(id))
    }

    /// As in SFTP version 3, renaming onto an existing file fails.
    async fn rename(&mut self, id: u32, old: String, new: String) -> Result<Status, Self::Error> {
        self.shared.counters.renames.fetch_add(1, Ordering::SeqCst);
        let mut st = self.lock();
        if st.files.contains_key(&new) {
            return Err(StatusCode::Failure);
        }
        let data = st.files.remove(&old).ok_or(StatusCode::NoSuchFile)?;
        st.files.insert(new, data);
        Ok(ok(id))
    }

    async fn realpath(&mut self, id: u32, path: String) -> Result<Name, Self::Error> {
        let full = if path.starts_with('/') { path } else { "/".to_string() };
        Ok(Name { id, files: vec![russh_sftp::protocol::File::dummy(full)] })
    }
}
