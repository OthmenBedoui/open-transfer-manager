//! The command line.
use otm_core::destination::{Destination, S3Destination};
use otm_core::engine::{download, Options};
use otm_core::relay::{put, relay, RelayOptions};
use otm_core::s3::{Client, Config};
use otm_core::sftp::{parse_address, HostKeys, Login, SftpDestination, Target};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

const USAGE: &str = "usage:
  otm get URL FILE
  otm relay URL TARGET [--work FILE] [--keep-local] [target options]
  otm put FILE TARGET [target options]

TARGET is s3://BUCKET/KEY or sftp://USER@HOST[:PORT]/PATH

S3 options:
  --endpoint URL      required
  --region NAME       default us-east-1
  Keys come from AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY.

SFTP options:
  --identity FILE     private key, default ~/.ssh/id_ed25519
  --host-key SHA256:  trust only this server key
  --accept-new-host-key  trust a first-seen key and remember it in ~/.ssh/known_hosts
  --no-verify         don't read pieces back to compare them
  --overwrite         replace a file that already exists
  The password comes from OTM_SFTP_PASSWORD, a key's passphrase from OTM_SFTP_KEY_PASSPHRASE.
  Without these, the server must already be in ~/.ssh/known_hosts.";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("get") if args.len() == 3 => get(&args[1], Path::new(&args[2])),
        Some("relay") if args.len() >= 3 => send(&args[1..], true),
        Some("put") if args.len() >= 3 => send(&args[1..], false),
        _ => stop_with(USAGE),
    }
}

fn get(url: &str, dest: &Path) {
    match download(url, dest, &Options::default(), Arc::new(AtomicBool::new(false))) {
        Ok(r) => println!("done: {} bytes in {:.1} s, at most {} connections", r.size, r.seconds, r.peak_connections),
        Err(e) => {
            eprintln!("failed: {}", e);
            std::process::exit(1);
        }
    }
}

fn stop_with(message: &str) -> ! {
    eprintln!("{}", message);
    std::process::exit(2);
}

fn home() -> PathBuf {
    std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from).unwrap_or_default()
}

#[derive(Default)]
struct Flags {
    endpoint: Option<String>,
    region: Option<String>,
    work: Option<PathBuf>,
    keep_local: bool,
    identity: Option<PathBuf>,
    host_key: Option<String>,
    accept_new: bool,
    no_verify: bool,
    overwrite: bool,
}

fn read_flags(args: &[String]) -> Flags {
    let mut f = Flags::default();
    let mut rest = args.iter();
    while let Some(flag) = rest.next() {
        let mut value = || rest.next().cloned().unwrap_or_else(|| stop_with(USAGE));
        match flag.as_str() {
            "--endpoint" => f.endpoint = Some(value()),
            "--region" => f.region = Some(value()),
            "--work" => f.work = Some(PathBuf::from(value())),
            "--identity" => f.identity = Some(PathBuf::from(value())),
            "--host-key" => f.host_key = Some(value()),
            "--keep-local" => f.keep_local = true,
            "--accept-new-host-key" => f.accept_new = true,
            "--no-verify" => f.no_verify = true,
            "--overwrite" => f.overwrite = true,
            _ => stop_with(USAGE),
        }
    }
    f
}

/// Secrets only ever come from the environment, where other programs can't read them in a process list.
fn destination(target: &str, f: &Flags) -> (Arc<dyn Destination>, String) {
    if let Some(rest) = target.strip_prefix("s3://") {
        let Some((bucket, key)) = rest.split_once('/').filter(|(b, k)| !b.is_empty() && !k.is_empty()) else { stop_with(USAGE) };
        let Some(endpoint) = f.endpoint.clone() else { stop_with("s3 needs --endpoint") };
        let (Ok(access_key), Ok(secret_key)) = (std::env::var("AWS_ACCESS_KEY_ID"), std::env::var("AWS_SECRET_ACCESS_KEY")) else {
            stop_with("set AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY first");
        };
        let region = f.region.clone().unwrap_or_else(|| "us-east-1".into());
        let client = Client::new(Config { endpoint, region, access_key, secret_key }).unwrap_or_else(|e| stop_with(&e.to_string()));
        return (Arc::new(S3Destination::new(Arc::new(client), bucket, key)), key.to_string());
    }
    let Some((host, port, user, path)) = parse_address(target) else { stop_with(USAGE) };
    let known = home().join(".ssh").join("known_hosts");
    let host_keys = match (&f.host_key, f.accept_new) {
        (Some(k), _) => HostKeys::Pinned(k.clone()),
        (None, true) => HostKeys::AcceptNew(known),
        (None, false) => HostKeys::KnownHosts(known),
    };
    let login = match std::env::var("OTM_SFTP_PASSWORD") {
        Ok(p) => Login::Password(p),
        Err(_) => Login::KeyFile {
            path: f.identity.clone().unwrap_or_else(|| home().join(".ssh").join("id_ed25519")),
            passphrase: std::env::var("OTM_SFTP_KEY_PASSPHRASE").ok(),
        },
    };
    let t = Target { host, port, user, path: path.clone(), login, host_keys, verify: !f.no_verify, overwrite: f.overwrite };
    let dest = SftpDestination::new(t).unwrap_or_else(|e| stop_with(&e.to_string()));
    (Arc::new(dest), path)
}

fn send(args: &[String], is_relay: bool) {
    let flags = read_flags(&args[2..]);
    let (dest, name) = destination(&args[1], &flags);
    let opts = RelayOptions { keep_local: flags.keep_local, ..RelayOptions::default() };
    let stop = Arc::new(AtomicBool::new(false));
    let result = if is_relay {
        let work = flags.work.clone().unwrap_or_else(|| PathBuf::from(name.rsplit('/').next().unwrap_or("relay.bin")));
        relay(&args[0], &work, dest, &opts, stop)
    } else {
        put(Path::new(&args[0]), dest, &opts, stop)
    };
    match result {
        Ok(r) => println!("stored: {} bytes in {:.1} s, {} parts", r.stored_size, r.seconds, r.parts),
        Err(e) => {
            eprintln!("failed: {}", e);
            std::process::exit(1);
        }
    }
}
