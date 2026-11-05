//! The command line: `otm get URL FILE`.
use otm_core::engine::{download, Options};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

const USAGE: &str = "usage: otm get URL FILE";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("get") if args.len() == 3 => get(&args[1], Path::new(&args[2])),
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
