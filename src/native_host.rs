//! Native messaging host: the bridge between the browser extension and the app.
//! The browser runs `otm native-host` and talks over stdin/stdout with 4-byte length-prefixed JSON.
//!
//! Requests carry an "id"; answers repeat it: {"id":1,"ok":true,"data":...}
//!   ping · add {url, filename?, referer?, cookies?, media?, opts?} · list · formats {url}
//!   control {dl, action} · show · subscribe (pushes {"type":"downloads",...} every second)

use crate::api;
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const MAX_MESSAGE: u32 = 1024 * 1024;

fn requires_confirmation(_body: &Value) -> bool {
    true
}

pub fn read_message(r: &mut impl Read) -> Result<Option<Value>, String> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.to_string()),
    }
    let n = u32::from_ne_bytes(len);
    if n > MAX_MESSAGE {
        return Err("message too large".into());
    }
    let mut buf = vec![0u8; n as usize];
    r.read_exact(&mut buf).map_err(|e| e.to_string())?;
    serde_json::from_slice(&buf)
        .map(Some)
        .map_err(|e| e.to_string())
}

pub fn encode_message(v: &Value) -> Vec<u8> {
    let data = v.to_string().into_bytes();
    let mut out = (data.len() as u32).to_ne_bytes().to_vec();
    out.extend(data);
    out
}

fn send(out: &Mutex<std::io::Stdout>, v: &Value) {
    let mut o = out.lock().unwrap_or_else(|e| e.into_inner());
    let _ = o.write_all(&encode_message(v));
    let _ = o.flush();
}

/// Start the app in the background if needed. True when it answers.
pub fn launch_app() -> bool {
    if api::is_running() {
        return true;
    }
    if let Ok(me) = std::env::current_exe() {
        let mut cmd = std::process::Command::new(me);
        cmd.arg("--from-browser")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0000_0008 | 0x0000_0200); // DETACHED_PROCESS | NEW_PROCESS_GROUP
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        let _ = cmd.spawn();
    }
    let end = Instant::now() + Duration::from_secs(15);
    while Instant::now() < end {
        if api::is_running() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    false
}

fn handle(msg: &Value) -> Result<Value, String> {
    let t = Duration::from_secs(15);
    match msg.get("cmd").and_then(Value::as_str).unwrap_or("") {
        "ping" => Ok(json!({"host": env!("CARGO_PKG_VERSION"), "app_running": api::is_running()})),
        "add" => {
            if !launch_app() {
                return Err("Could not start Open Transfer Manager".into());
            }
            let capability = api::request("GET", "/api/ping", None, t)?;
            if capability["capabilities"]["download_confirmation"].as_bool() != Some(true) {
                return Err("Ouvrez ou redémarrez l’application desktop OTM 0.2.2 ou ultérieure pour confirmer ce téléchargement.".into());
            }
            let mut body = json!({});
            for k in [
                "url",
                "filename",
                "referer",
                "cookies",
                "user_agent",
                "media",
                "quality",
                "opts",
                "start",
                "focus",
                "confirm",
            ] {
                if let Some(v) = msg.get(k) {
                    body[k] = v.clone();
                }
            }
            // Regular browser requests require confirmation, including old extensions.
            body["confirm"] = json!(requires_confirmation(&body));
            api::request("POST", "/api/add", Some(body), t)
        }
        "formats" => {
            if !launch_app() {
                return Err("Could not start Open Transfer Manager".into());
            }
            api::request(
                "POST",
                "/api/formats",
                Some(json!({"url": msg.get("url")})),
                Duration::from_secs(60),
            )
        }
        "list" => {
            if !api::is_running() {
                return Ok(json!({"app_running": false, "downloads": []}));
            }
            let mut v = api::request("GET", "/api/downloads", None, t)?;
            v["app_running"] = json!(true);
            Ok(v)
        }
        "control" => api::request(
            "POST",
            "/api/control",
            Some(json!({"id": msg.get("dl"), "action": msg.get("action")})),
            t,
        ),
        "show" => {
            if !launch_app() {
                return Err("Could not start Open Transfer Manager".into());
            }
            api::request("POST", "/api/show", None, t)
        }
        other => Err(format!("unknown command {other:?}")),
    }
}

pub fn main() -> i32 {
    let out = Arc::new(Mutex::new(std::io::stdout()));
    let stop = Arc::new(AtomicBool::new(false));
    let mut pusher_started = false;
    let mut workers = Vec::new();
    let mut stdin = std::io::stdin().lock();
    while let Ok(Some(msg)) = read_message(&mut stdin) {
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        if msg.get("cmd").and_then(Value::as_str) == Some("subscribe") {
            if !pusher_started {
                pusher_started = true;
                let out = out.clone();
                let stop = stop.clone();
                std::thread::spawn(move || {
                    while !stop.load(Ordering::SeqCst) {
                        let mut v = handle(&json!({"cmd": "list"})).unwrap_or_else(
                            |e| json!({"app_running": false, "downloads": [], "error": e}),
                        );
                        v["type"] = json!("downloads");
                        send(&out, &v);
                        std::thread::sleep(Duration::from_secs(1));
                    }
                });
            }
            send(&out, &json!({"id": id, "ok": true}));
            continue;
        }
        let out = out.clone();
        workers.push(std::thread::spawn(move || {
            let reply = match handle(&msg) {
                Ok(data) => json!({"id": id, "ok": true, "data": data}),
                Err(e) => json!({"id": id, "ok": false, "error": e}),
            };
            send(&out, &reply);
        }));
        workers.retain(|w: &std::thread::JoinHandle<()>| !w.is_finished());
    }
    stop.store(true, Ordering::SeqCst);
    for w in workers {
        let _ = w.join(); // one-shot calls: let the answer go out before exiting
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn audio_enrichment_does_not_bypass_regular_confirmation() {
        assert!(requires_confirmation(&json!({"confirm": false})));
        assert!(requires_confirmation(
            &json!({"opts":{"quality":"audio","music_enriched":false}})
        ));
        assert!(requires_confirmation(
            &json!({"opts":{"quality":"audio","music_enriched":true}})
        ));
    }

    #[test]
    fn framing_roundtrip() {
        let v = json!({"id": 7, "cmd": "ping", "text": "héllo"});
        let bytes = encode_message(&v);
        assert_eq!(read_message(&mut bytes.as_slice()).unwrap(), Some(v));
        assert_eq!(read_message(&mut [].as_slice()).unwrap(), None);
        let big = (10u32.pow(8)).to_ne_bytes();
        assert!(read_message(&mut big.as_slice()).is_err());
    }
}
