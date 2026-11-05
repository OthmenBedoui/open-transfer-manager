//! The command line.
use otm_core::engine::{download, Options};
use otm_core::relay::{relay, RelayOptions};
use otm_core::s3::{Client, Config};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

const USAGE: &str = "usage:
  otm get URL FILE
  otm relay URL s3://BUCKET/KEY --endpoint URL [--region NAME] [--work FILE] [--keep-local]

The relay reads its keys from AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY.";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("get") if args.len() == 3 => get(&args[1], Path::new(&args[2])),
        Some("relay") if args.len() >= 3 => relay_command(&args[1..]),
        _ => {
            eprintln!("{}", USAGE);
            std::process::exit(2);
        }
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

fn relay_command(args: &[String]) {
    let url = &args[0];
    let Some(target) = args[1].strip_prefix("s3://") else { stop_with(USAGE) };
    let Some((bucket, key)) = target.split_once('/').filter(|(b, k)| !b.is_empty() && !k.is_empty()) else { stop_with(USAGE) };
    let (mut endpoint, mut region, mut work, mut keep_local) = (None, "us-east-1".to_string(), None, false);
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--endpoint" => {
                i += 1;
                endpoint = args.get(i).cloned();
            }
            "--region" => {
                i += 1;
                region = args.get(i).cloned().unwrap_or(region);
            }
            "--work" => {
                i += 1;
                work = args.get(i).map(std::path::PathBuf::from);
            }
            "--keep-local" => keep_local = true,
            _ => stop_with(USAGE),
        }
        i += 1;
    }
    let Some(endpoint) = endpoint else { stop_with(USAGE) };
    // Keys never go on the command line, where other programs could read them.
    let (Ok(access_key), Ok(secret_key)) = (std::env::var("AWS_ACCESS_KEY_ID"), std::env::var("AWS_SECRET_ACCESS_KEY")) else {
        stop_with("set AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY first");
    };
    let work = work.unwrap_or_else(|| std::path::PathBuf::from(key.rsplit('/').next().unwrap_or("relay.bin")));
    let client = match Client::new(Config { endpoint, region, access_key, secret_key }) {
        Ok(c) => Arc::new(c),
        Err(e) => stop_with(&e.to_string()),
    };
    let opts = RelayOptions { keep_local, ..RelayOptions::default() };
    match relay(url, &work, client, bucket, key, &opts, Arc::new(AtomicBool::new(false))) {
        Ok(r) => println!("stored: {} bytes in {:.1} s, {} parts", r.stored_size, r.seconds, r.parts),
        Err(e) => {
            eprintln!("failed: {}", e);
            std::process::exit(1);
        }
    }
}
