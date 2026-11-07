//! Local control API on 127.0.0.1, used by the browser bridge and by a second `otm` launch.
//!
//! Security: loopback only; every call except /api/ping needs the secret token from
//! runtime.json (mode 600); the Host header must be 127.0.0.1/localhost (DNS rebinding);
//! no CORS headers are ever sent, so web pages cannot read answers or send the token.

use crate::manager::{Event, Manager, lock};
use crate::model::*;
use crate::{media, paths};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

pub const DEFAULT_PORT: u16 = 51737;
const MAX_BODY: u64 = 256 * 1024;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Runtime {
    pub port: u16,
    pub token: String,
    pub pid: u32,
    pub version: String,
}

pub fn runtime_path() -> PathBuf {
    paths::data_dir().join("runtime.json")
}

pub fn read_runtime() -> Option<Runtime> {
    serde_json::from_str(&std::fs::read_to_string(runtime_path()).ok()?).ok()
}

// ------------------------------------------------------------------ client

pub fn request(
    method: &str,
    path: &str,
    body: Option<Value>,
    timeout: Duration,
) -> Result<Value, String> {
    let rt = read_runtime().ok_or("Open Transfer Manager is not running")?;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .http_status_as_error(false)
        .proxy(None)
        .build()
        .into();
    let url = format!("http://127.0.0.1:{}{}", rt.port, path);
    let resp = if method == "GET" {
        agent.get(&url).header("X-OTM-Token", &rt.token).call()
    } else {
        agent
            .post(&url)
            .header("X-OTM-Token", &rt.token)
            .header("Content-Type", "application/json")
            .send(body.unwrap_or(json!({})).to_string())
    };
    let resp = resp.map_err(|e| e.to_string())?;
    let status = resp.status().as_u16();
    let text = resp.into_body().read_to_string().unwrap_or_default();
    let v: Value = serde_json::from_str(&text).unwrap_or(json!({}));
    if status >= 400 {
        return Err(v
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("request failed")
            .to_string());
    }
    Ok(v)
}

pub fn is_running() -> bool {
    request("GET", "/api/ping", None, Duration::from_millis(1500))
        .map(|v| v.get("app").and_then(Value::as_str) == Some("otm"))
        .unwrap_or(false)
}

// ------------------------------------------------------------------ server

pub struct ApiServer {
    server: Arc<tiny_http::Server>,
    pub port: u16,
}

impl ApiServer {
    pub fn start(mgr: Manager) -> Result<ApiServer, String> {
        let mut server = None;
        for port in (DEFAULT_PORT..DEFAULT_PORT + 10).chain([0]) {
            if let Ok(s) = tiny_http::Server::http(("127.0.0.1", port)) {
                server = Some(s);
                break;
            }
        }
        let server = Arc::new(server.ok_or("cannot listen on 127.0.0.1")?);
        let port = server.server_addr().to_ip().map(|a| a.port()).unwrap_or(0);
        let token = new_token();
        let rt = Runtime {
            port,
            token: token.clone(),
            pid: std::process::id(),
            version: env!("CARGO_PKG_VERSION").into(),
        };
        write_private(
            &runtime_path(),
            &serde_json::to_string(&rt).unwrap_or_default(),
        );
        let s2 = server.clone();
        std::thread::Builder::new()
            .name("otm-api".into())
            .spawn(move || {
                for req in s2.incoming_requests() {
                    let mgr = mgr.clone();
                    let token = token.clone();
                    std::thread::spawn(move || handle(req, &mgr, &token));
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(ApiServer { server, port })
    }

    pub fn stop(&self) {
        self.server.unblock();
        if read_runtime().is_some_and(|r| r.pid == std::process::id()) {
            let _ = std::fs::remove_file(runtime_path());
        }
    }
}

fn new_token() -> String {
    let mut b = [0u8; 32];
    let _ = getrandom::fill(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn write_private(path: &std::path::Path, text: &str) {
    let tmp = path.with_extension("json.tmp");
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)
        {
            let _ = f.write_all(text.as_bytes());
        }
    }
    #[cfg(not(unix))]
    let _ = std::fs::write(&tmp, text);
    let _ = std::fs::rename(&tmp, path);
}

fn respond(req: tiny_http::Request, code: u16, v: Value) {
    let body = v.to_string();
    let resp = tiny_http::Response::from_string(body)
        .with_status_code(code)
        .with_header(tiny_http::Header::from_bytes("Content-Type", "application/json").unwrap())
        .with_header(tiny_http::Header::from_bytes("Cache-Control", "no-store").unwrap());
    let _ = req.respond(resp);
}

fn constant_eq(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

fn handle(mut req: tiny_http::Request, mgr: &Manager, token: &str) {
    let header = |name: &'static str| {
        req.headers()
            .iter()
            .find(|h| h.field.equiv(name))
            .map(|h| h.value.as_str().to_string())
            .unwrap_or_default()
    };
    let host = header("Host");
    let host_name = host
        .rsplit_once(':')
        .map(|(h, _)| h)
        .unwrap_or(&host)
        .trim_matches(['[', ']']);
    if !["127.0.0.1", "localhost", "::1"].contains(&host_name) {
        return respond(req, 403, json!({"error": "bad host"}));
    }
    let path = req.url().to_string();
    let method = req.method().to_string();
    if path == "/api/ping" {
        return respond(
            req,
            200,
            json!({"app": "otm", "version": env!("CARGO_PKG_VERSION"), "capabilities": {"download_confirmation": mgr.has_confirmation_ui(), "music_enrichment": mgr.settings().music_tags}}),
        );
    }
    if !constant_eq(&header("X-OTM-Token"), token) {
        return respond(req, 401, json!({"error": "bad token"}));
    }
    if method == "OPTIONS" {
        return respond(req, 405, json!({"error": "not allowed"}));
    }
    let mut body = String::new();
    if method == "POST" {
        if req.body_length().unwrap_or(0) as u64 > MAX_BODY {
            return respond(req, 413, json!({"error": "request too large"}));
        }
        let _ = req.as_reader().take(MAX_BODY).read_to_string(&mut body);
    }
    let body: Value = if body.trim().is_empty() {
        json!({})
    } else {
        match serde_json::from_str(&body) {
            Ok(v) => v,
            Err(e) => return respond(req, 400, json!({"error": e.to_string()})),
        }
    };
    match route(&method, &path, &body, mgr) {
        Ok(v) => respond(req, 200, v),
        Err((code, e)) => respond(req, code, json!({"error": e})),
    }
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

fn route(method: &str, path: &str, body: &Value, mgr: &Manager) -> Result<Value, (u16, String)> {
    match (method, path) {
        ("GET", "/api/downloads") => {
            let list: Vec<Summary> = mgr
                .list()
                .iter()
                .map(|d| Summary::from(&*lock(d)))
                .collect();
            Ok(json!({"downloads": list, "total_rate": mgr.total_rate()}))
        }
        ("POST", "/api/add") => {
            let url = s(body, "url").trim();
            if !(url.starts_with("http://") || url.starts_with("https://")) {
                return Err((400, "only http(s) links can be downloaded".into()));
            }
            let mut headers = Vec::new();
            for (k, h) in [
                ("referer", "Referer"),
                ("cookies", "Cookie"),
                ("user_agent", "User-Agent"),
            ] {
                let v = s(body, k);
                if !v.is_empty() {
                    headers.push((h.to_string(), v.chars().take(8192).collect()));
                }
            }
            let media = body.get("media").and_then(Value::as_bool);
            let start = body.get("start").and_then(Value::as_bool).unwrap_or(true);
            let opts: Option<MediaOpts> = body
                .get("opts")
                .and_then(|o| serde_json::from_value(o.clone()).ok())
                .or_else(|| {
                    let q = s(body, "quality");
                    (!q.is_empty()).then(|| MediaOpts {
                        quality: q.to_string(),
                        ..mgr.settings().media
                    })
                });
            let media = if opts.is_some() && media.is_none() {
                Some(true)
            } else {
                media
            };
            let mut trusted = false;
            if body["confirm"].as_bool() == Some(true) {
                let request = DownloadRequest {
                    url: url.to_string(),
                    filename: s(body, "filename").to_string(),
                    headers: headers.clone(),
                    media,
                    opts: opts.clone(),
                };
                // sites the user chose to trust skip the confirmation window
                trusted = mgr.settings().trusts(&request.site());
                if !trusted {
                    if !mgr.has_confirmation_ui() {
                        return Err((
                            409,
                            "This instance is headless; open the desktop app to confirm downloads."
                                .into(),
                        ));
                    }
                    mgr.push_event(Event::ConfirmDownload(request));
                    mgr.push_event(Event::ShowWindow);
                    return Ok(json!({"ok": true, "pending_confirmation": true}));
                }
            }
            let dl = mgr.add_smart(url, s(body, "filename"), media, start, headers, opts);
            if body.get("focus").and_then(Value::as_bool) == Some(true) {
                mgr.push_event(Event::ShowWindow);
            }
            let sum = Summary::from(&*lock(&dl));
            Ok(json!({"ok": true, "download": sum, "trusted_site": trusted}))
        }
        ("POST", "/api/formats") => {
            let url = s(body, "url");
            let info = media::probe(mgr, url, &[]).map_err(|e| (502, e))?;
            Ok(serde_json::to_value(info).unwrap_or(json!({})))
        }
        ("POST", "/api/control") => {
            let id = body
                .get("id")
                .and_then(Value::as_u64)
                .ok_or((400, "missing id".to_string()))?;
            let dl = mgr.find(id).ok_or((404, "no such download".to_string()))?;
            match s(body, "action") {
                "pause" | "stop" => mgr.stop(&dl),
                "resume" | "start" => mgr.start(&dl),
                "remove" => mgr.remove(id, false),
                "open" => paths::open_path(&lock(&dl).path()),
                "reveal" => paths::reveal(&lock(&dl).path()),
                _ => return Err((400, "unknown action".into())),
            }
            Ok(json!({"ok": true}))
        }
        ("POST", "/api/show") => {
            mgr.push_event(Event::ShowWindow);
            if body["browser_settings"].as_bool() == Some(true) {
                mgr.push_event(Event::ShowBrowserSetup);
            }
            Ok(json!({"ok": true}))
        }
        ("POST", "/api/pause-all") => {
            mgr.stop_all();
            Ok(json!({"ok": true}))
        }
        _ => Err((404, "not found".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_confirmation_creates_no_download_until_approved() {
        let mgr = Manager::new(false);
        let body = json!({"url":"https://example.com/report.pdf", "filename":"تقرير العمل.pdf", "confirm":true, "cookies":"test=value"});
        assert_eq!(route("POST", "/api/add", &body, &mgr).unwrap_err().0, 409);
        mgr.enable_confirmation_ui();
        let response = route("POST", "/api/add", &body, &mgr).unwrap();
        assert_eq!(response["pending_confirmation"], true);
        assert!(response.get("download").is_none());
        assert!(mgr.list().is_empty());
        let requests: Vec<_> = mgr
            .take_events()
            .into_iter()
            .filter_map(|e| match e {
                Event::ConfirmDownload(r) => Some(r),
                _ => None,
            })
            .collect();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].filename, "تقرير العمل.pdf");
        assert_eq!(
            requests[0].headers,
            vec![("Cookie".into(), "test=value".into())]
        );
    }

    #[test]
    fn trusted_site_skips_confirmation_other_sites_still_ask() {
        let dir = std::env::temp_dir().join(format!("otm-trust-{}", std::process::id()));
        let mgr = Manager::new(false);
        mgr.update_settings(|s| {
            s.base_folder = dir.clone();
            s.categories = crate::model::default_categories(&dir);
            s.trust_site("example.com");
        });
        mgr.enable_confirmation_ui();
        // link sent from a trusted page: added straight away, not started by the test
        let body = json!({"url":"https://cdn.example.net/a.zip", "referer":"https://www.example.com/page",
                          "confirm":true, "start":false});
        let r = route("POST", "/api/add", &body, &mgr).unwrap();
        assert_eq!(r["trusted_site"], true);
        assert!(r.get("download").is_some());
        assert_eq!(mgr.list().len(), 1);
        // a page from another site still asks first
        let other = json!({"url":"https://cdn.example.net/b.zip", "referer":"https://other.org/", "confirm":true});
        let r = route("POST", "/api/add", &other, &mgr).unwrap();
        assert_eq!(r["pending_confirmation"], true);
        assert_eq!(mgr.list().len(), 1);
        // a look-alike domain is not trusted
        let fake = json!({"url":"https://cdn.example.net/c.zip", "referer":"https://example.com.evil.net/", "confirm":true});
        assert_eq!(
            route("POST", "/api/add", &fake, &mgr).unwrap()["pending_confirmation"],
            true
        );
        assert_eq!(mgr.list().len(), 1);
    }

    #[test]
    fn api_end_to_end() {
        let dir = std::env::temp_dir().join(format!("otm-api-{}", std::process::id()));
        // SAFETY: tests in this module run in one thread for this variable
        unsafe { std::env::set_var("OTM_DATA_DIR", &dir) };
        let mgr = Manager::new(false);
        let srv = ApiServer::start(mgr.clone()).unwrap();
        assert!(is_running());
        let rt = read_runtime().unwrap();
        // no token -> 401
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .build()
            .into();
        let r = agent
            .get(format!("http://127.0.0.1:{}/api/downloads", rt.port))
            .call()
            .unwrap();
        assert_eq!(r.status().as_u16(), 401);
        // bad host -> 403
        let r = agent
            .get(format!("http://127.0.0.1:{}/api/ping", rt.port))
            .header("Host", "evil.example")
            .call()
            .unwrap();
        assert_eq!(r.status().as_u16(), 403);
        // add, list, remove
        let v = request("POST", "/api/add", Some(json!({"url": "https://example.com/files/a.zip", "start": false, "referer": "https://example.com/"})), Duration::from_secs(5)).unwrap();
        let id = v["download"]["id"].as_u64().unwrap();
        assert_eq!(v["download"]["category"], "Archives");
        assert_eq!(
            lock(&mgr.find(id).unwrap()).headers[0].1,
            "https://example.com/"
        );
        let v = request(
            "POST",
            "/api/add",
            Some(json!({"url": "https://youtu.be/abc", "start": false})),
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(v["download"]["kind"], "media");
        let list = request("GET", "/api/downloads", None, Duration::from_secs(5)).unwrap();
        assert_eq!(list["downloads"].as_array().unwrap().len(), 2);
        request(
            "POST",
            "/api/control",
            Some(json!({"id": id, "action": "remove"})),
            Duration::from_secs(5),
        )
        .unwrap();
        assert!(mgr.find(id).is_none());
        assert!(
            request(
                "POST",
                "/api/add",
                Some(json!({"url": "file:///etc/passwd"})),
                Duration::from_secs(5)
            )
            .is_err()
        );
        srv.stop();
    }
}
