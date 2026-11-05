//! A small client for S3-compatible storage: multipart uploads, signed with AWS Signature V4.
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const MIN_PART: u64 = 5 * 1024 * 1024;
const MAX_PARTS: u64 = 10_000;

#[derive(Clone, Debug)]
pub struct Config {
    /// For example `https://s3.eu-west-1.amazonaws.com` or `http://127.0.0.1:9000`.
    pub endpoint: String,
    pub region: String,
    pub access_key: String,
    pub secret_key: String,
}

#[derive(Debug, PartialEq)]
pub enum S3Error {
    /// Worth trying again, after this long if the server said so.
    Retry(String, Option<u64>),
    Fatal(String),
}

impl std::fmt::Display for S3Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            S3Error::Retry(m, _) | S3Error::Fatal(m) => write!(f, "{}", m),
        }
    }
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex(&Sha256::digest(data))
}

fn hmac(key: &[u8], data: &str) -> Vec<u8> {
    let mut m = Hmac::<Sha256>::new_from_slice(key).expect("any key length works");
    m.update(data.as_bytes());
    m.finalize().into_bytes().to_vec()
}

/// Percent-encoding as Signature V4 wants it. `/` stays only in object paths.
pub fn encode(text: &str, keep_slash: bool) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b'/' if keep_slash => out.push('/'),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// (YYYYMMDD, YYYYMMDDTHHMMSSZ) in UTC.
fn stamps(unix: u64) -> (String, String) {
    let days = (unix / 86400) as i64;
    let (h, m, s) = (unix / 3600 % 24, unix / 60 % 60, unix % 60);
    // Calendar date from a day count, valid for all dates this program can meet.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let date = format!("{:04}{:02}{:02}", year, month, day);
    let full = format!("{}T{:02}{:02}{:02}Z", date, h, m, s);
    (date, full)
}

/// The `Authorization` header. `headers` must be lowercase and sorted, and hold everything signed.
pub fn authorization(cfg: &Config, method: &str, path: &str, query: &str, headers: &[(&str, &str)], payload_sha: &str, date: &str, stamp: &str) -> String {
    let canonical_headers: String = headers.iter().map(|(k, v)| format!("{}:{}\n", k, v.trim())).collect();
    let signed: Vec<&str> = headers.iter().map(|(k, _)| *k).collect();
    let signed = signed.join(";");
    let request = format!("{}\n{}\n{}\n{}\n{}\n{}", method, path, query, canonical_headers, signed, payload_sha);
    let scope = format!("{}/{}/s3/aws4_request", date, cfg.region);
    let to_sign = format!("AWS4-HMAC-SHA256\n{}\n{}\n{}", stamp, scope, sha256_hex(request.as_bytes()));
    let k = hmac(format!("AWS4{}", cfg.secret_key).as_bytes(), date);
    let k = hmac(&k, &cfg.region);
    let k = hmac(&k, "s3");
    let k = hmac(&k, "aws4_request");
    let signature = hex(&hmac(&k, &to_sign));
    format!("AWS4-HMAC-SHA256 Credential={}/{},SignedHeaders={},Signature={}", cfg.access_key, scope, signed, signature)
}

/// Part size for a file: large enough to stay under the limit on the number of parts.
pub fn part_size_for(size: u64) -> u64 {
    let needed = size.div_ceil(MAX_PARTS - 1);
    let chosen = needed.max(8 * 1024 * 1024);
    // Whole mebibytes.
    chosen.div_ceil(1024 * 1024) * 1024 * 1024
}

fn tag<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{}>", name);
    let close = format!("</{}>", name);
    let a = xml.find(&open)? + open.len();
    let b = xml[a..].find(&close)? + a;
    Some(&xml[a..b])
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

pub struct Client {
    cfg: Config,
    agent: ureq::Agent,
    host: String,
    base_path: String,
}

impl Client {
    pub fn new(cfg: Config) -> Result<Client, S3Error> {
        let u = url::Url::parse(&cfg.endpoint).map_err(|e| S3Error::Fatal(format!("bad endpoint: {}", e)))?;
        let name = u.host_str().ok_or_else(|| S3Error::Fatal("the endpoint has no host".into()))?;
        let host = match u.port() {
            Some(p) => format!("{}:{}", name, p),
            None => name.to_string(),
        };
        let base_path = u.path().trim_end_matches('/').to_string();
        let tls = native_tls::TlsConnector::new().map_err(|e| S3Error::Fatal(e.to_string()))?;
        let agent = ureq::AgentBuilder::new()
            .tls_connector(std::sync::Arc::new(tls))
            .timeout_connect(Duration::from_secs(15))
            .timeout_read(Duration::from_secs(120))
            .timeout_write(Duration::from_secs(120))
            .build();
        Ok(Client { cfg, agent, host, base_path })
    }

    /// Sends one signed request and returns the status-checked answer.
    fn call(&self, method: &str, bucket: &str, key: &str, query: &str, body: &[u8]) -> Result<ureq::Response, S3Error> {
        let path = format!("{}/{}/{}", self.base_path, encode(bucket, false), encode(key, true));
        let payload_sha = sha256_hex(body);
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let (date, stamp) = stamps(now);
        let headers = [("host", self.host.as_str()), ("x-amz-content-sha256", payload_sha.as_str()), ("x-amz-date", stamp.as_str())];
        let auth = authorization(&self.cfg, method, &path, query, &headers, &payload_sha, &date, &stamp);
        let endpoint = self.cfg.endpoint.trim_end_matches('/');
        let origin = endpoint.strip_suffix(self.base_path.as_str()).unwrap_or(endpoint);
        let url = if query.is_empty() { format!("{}{}", origin, path) } else { format!("{}{}?{}", origin, path, query) };
        let request = self.agent.request(method, &url).set("Authorization", &auth).set("x-amz-content-sha256", &payload_sha).set("x-amz-date", &stamp);
        match request.send_bytes(body) {
            Ok(r) => Ok(r),
            Err(ureq::Error::Status(code, r)) => {
                let wait = r.header("Retry-After").and_then(|v| v.trim().parse().ok());
                let mut text = String::new();
                let _ = r.into_reader().take(4096).read_to_string(&mut text);
                let code_name = tag(&text, "Code").unwrap_or("").to_string();
                let message = format!("the storage answered HTTP {} {}", code, code_name);
                if code >= 500 || code == 429 || code == 408 {
                    Err(S3Error::Retry(message, wait))
                } else {
                    Err(S3Error::Fatal(message))
                }
            }
            Err(e) => Err(S3Error::Retry(e.to_string(), None)),
        }
    }

    pub fn create_multipart(&self, bucket: &str, key: &str) -> Result<String, S3Error> {
        let r = self.call("POST", bucket, key, "uploads=", b"")?;
        let mut text = String::new();
        r.into_reader().take(1 << 20).read_to_string(&mut text).map_err(|e| S3Error::Retry(e.to_string(), None))?;
        tag(&text, "UploadId").map(str::to_string).ok_or_else(|| S3Error::Retry("no upload id in the answer".into(), None))
    }

    /// Returns the part's ETag. The signed SHA-256 makes the storage refuse damaged data.
    pub fn upload_part(&self, bucket: &str, key: &str, upload_id: &str, number: u32, data: &[u8]) -> Result<String, S3Error> {
        let query = format!("partNumber={}&uploadId={}", number, encode(upload_id, false));
        let r = self.call("PUT", bucket, key, &query, data)?;
        r.header("ETag").map(str::to_string).ok_or_else(|| S3Error::Retry("no ETag in the answer".into(), None))
    }

    pub fn complete(&self, bucket: &str, key: &str, upload_id: &str, parts: &[(u32, String)]) -> Result<(), S3Error> {
        let mut body = String::from("<CompleteMultipartUpload>");
        for (n, etag) in parts {
            body.push_str(&format!("<Part><PartNumber>{}</PartNumber><ETag>{}</ETag></Part>", n, escape_xml(etag)));
        }
        body.push_str("</CompleteMultipartUpload>");
        let query = format!("uploadId={}", encode(upload_id, false));
        let r = self.call("POST", bucket, key, &query, body.as_bytes())?;
        let mut text = String::new();
        r.into_reader().take(1 << 20).read_to_string(&mut text).map_err(|e| S3Error::Retry(e.to_string(), None))?;
        // This call can answer 200 and still report a failure in its body.
        if text.contains("<Error>") {
            return Err(S3Error::Retry(format!("completing failed: {}", tag(&text, "Code").unwrap_or("unknown")), None));
        }
        Ok(())
    }

    pub fn abort(&self, bucket: &str, key: &str, upload_id: &str) -> Result<(), S3Error> {
        let query = format!("uploadId={}", encode(upload_id, false));
        self.call("DELETE", bucket, key, &query, b"").map(|_| ())
    }

    pub fn put_object(&self, bucket: &str, key: &str, data: &[u8]) -> Result<(), S3Error> {
        self.call("PUT", bucket, key, "", data).map(|_| ())
    }

    /// The stored object's size.
    pub fn head_size(&self, bucket: &str, key: &str) -> Result<u64, S3Error> {
        let r = self.call("HEAD", bucket, key, "", b"")?;
        r.header("Content-Length").and_then(|v| v.parse().ok()).ok_or_else(|| S3Error::Retry("no size in the answer".into(), None))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The worked example from the AWS documentation for Signature V4 on S3.
    #[test]
    fn signature_matches_the_published_example() {
        let cfg = Config {
            endpoint: "https://examplebucket.s3.amazonaws.com".into(),
            region: "us-east-1".into(),
            access_key: "AKIAIOSFODNN7EXAMPLE".into(),
            secret_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into(),
        };
        let empty = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        let headers = [
            ("host", "examplebucket.s3.amazonaws.com"),
            ("range", "bytes=0-9"),
            ("x-amz-content-sha256", empty),
            ("x-amz-date", "20130524T000000Z"),
        ];
        let a = authorization(&cfg, "GET", "/test.txt", "", &headers, empty, "20130524", "20130524T000000Z");
        assert_eq!(
            a,
            "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request,SignedHeaders=host;range;x-amz-content-sha256;x-amz-date,Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
        );
    }

    #[test]
    fn dates_are_right() {
        assert_eq!(stamps(0), ("19700101".into(), "19700101T000000Z".into()));
        assert_eq!(stamps(1_369_353_600), ("20130524".into(), "20130524T000000Z".into()));
        assert_eq!(stamps(1_709_210_096).0, "20240229");
    }

    #[test]
    fn names_are_encoded() {
        assert_eq!(encode("a b/é.mp4", true), "a%20b/%C3%A9.mp4");
        assert_eq!(encode("a/b+c", false), "a%2Fb%2Bc");
    }

    #[test]
    fn parts_stay_within_the_limits() {
        assert_eq!(part_size_for(1), 8 * 1024 * 1024);
        let huge = 5u64 * 1024 * 1024 * 1024 * 1024;
        let p = part_size_for(huge);
        assert!(huge.div_ceil(p) <= MAX_PARTS, "{}", huge.div_ceil(p));
        assert!(p >= MIN_PART);
    }

    #[test]
    fn xml_is_read_and_written_safely() {
        assert_eq!(tag("<a><UploadId>x1</UploadId></a>", "UploadId"), Some("x1"));
        assert_eq!(tag("<a></a>", "UploadId"), None);
        assert_eq!(escape_xml("\"ab&c\""), "&quot;ab&amp;c&quot;");
    }
}
