//! Optional song enrichment through LRCLIB and MusicBrainz / Cover Art Archive.
use crate::manager::Manager;
use id3::{
    Tag, TagLike, Version,
    frame::{Lyrics, Picture, PictureType},
};
use serde_json::Value;
use std::{io::Read, path::Path};

fn client() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(8)))
        .user_agent(concat!(
            "OpenTransferManager/",
            env!("CARGO_PKG_VERSION"),
            " (https://github.com/OthmenBedoui/open-transfer-manager)"
        ))
        .build()
        .into()
}

fn query(value: &str) -> String {
    value
        .as_bytes()
        .iter()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(b) {
                char::from(*b).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

fn json(agent: &ureq::Agent, url: &str) -> Option<Value> {
    let response = agent.get(url).call().ok()?;
    let mut bytes = Vec::new();
    response
        .into_body()
        .into_reader()
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > 1024 * 1024 {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}

fn normalized(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn identity(value: &Value) -> Option<(String, String)> {
    let mut title = value["track"]
        .as_str()
        .or(value["title"].as_str())?
        .trim()
        .to_string();
    let mut artist = value["artist"]
        .as_str()
        .or_else(|| value["artists"].as_array()?.first()?.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if artist.is_empty() {
        let (a, t) = title.split_once(" - ")?;
        artist = a.trim().into();
        title = t.trim().into();
    } else if let Some((a, t)) = title.split_once(" - ")
        && normalized(a) == normalized(&artist)
    {
        title = t.trim().into();
    }
    // Strip presentation suffixes only; retain live/remix/version qualifiers.
    for suffix in [
        "(Official Music Video)",
        "(Official Video)",
        "(Official Audio)",
        "(Lyrics)",
        "[Official Video]",
        "[Official Audio]",
    ] {
        if title.to_lowercase().ends_with(&suffix.to_lowercase()) {
            title.truncate(title.len() - suffix.len());
            title = title.trim().into();
        }
    }
    if title.is_empty() || artist.is_empty() {
        return None;
    }
    Some((title, artist))
}

fn matching_lyrics(record: &Value, title: &str, artist: &str, duration: f64) -> bool {
    normalized(record["trackName"].as_str().unwrap_or("")) == normalized(title)
        && normalized(record["artistName"].as_str().unwrap_or("")) == normalized(artist)
        && record["duration"]
            .as_f64()
            .is_some_and(|d| (d - duration).abs() <= 3.0)
        && record["instrumental"].as_bool() != Some(true)
}

/// Network failures and missing catalogue entries are optional, never download errors.
pub fn resolve(value: &Value) -> Option<Song> {
    if value["entries"].is_array() {
        return None;
    }
    let (title, artist) = identity(value)?;
    let duration = value["duration"]
        .as_f64()
        .filter(|d| d.is_finite() && *d > 0.0)?;
    let agent = client();
    let url = format!(
        "https://lrclib.net/api/get?track_name={}&artist_name={}&duration={duration}",
        query(&title),
        query(&artist)
    );
    let record = json(&agent, &url).filter(|r| matching_lyrics(r, &title, &artist, duration));
    let mut song = Song {
        title,
        artist,
        album: record
            .as_ref()
            .and_then(|r| r["albumName"].as_str())
            .or(value["album"].as_str())
            .unwrap_or("")
            .into(),
        lyrics: record
            .as_ref()
            .and_then(|r| {
                r["plainLyrics"]
                    .as_str()
                    .filter(|s| !s.trim().is_empty())
                    .or(r["syncedLyrics"].as_str())
            })
            .filter(|s| !s.trim().is_empty() && s.len() <= 256 * 1024)
            .unwrap_or("")
            .into(),
        cover_url: String::new(),
    };
    if !song.album.is_empty() {
        // Serialize MusicBrainz lookups and respect its one request/second limit.
        static LAST: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);
        let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(t) = *last {
            std::thread::sleep(std::time::Duration::from_secs(1).saturating_sub(t.elapsed()));
        }
        *last = Some(std::time::Instant::now());
        let terms = format!(
            "release:\"{}\" AND artist:\"{}\"",
            song.album.replace(['"', '\\'], " "),
            song.artist.replace(['"', '\\'], " ")
        );
        if let Some(result) = json(
            &agent,
            &format!(
                "https://musicbrainz.org/ws/2/release/?query={}&fmt=json&limit=5",
                query(&terms)
            ),
        ) {
            for release in result["releases"].as_array().into_iter().flatten() {
                let same_album =
                    normalized(release["title"].as_str().unwrap_or("")) == normalized(&song.album);
                let same_artist = release["artist-credit"].as_array().is_some_and(|a| {
                    a.iter().any(|c| {
                        normalized(c["artist"]["name"].as_str().unwrap_or(""))
                            == normalized(&song.artist)
                    })
                });
                if same_album
                    && same_artist
                    && let Some(id) = release["id"].as_str().filter(|id| {
                        id.len() == 36 && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
                    })
                {
                    song.cover_url = format!("https://coverartarchive.org/release/{id}/front-500");
                    break;
                }
            }
        }
    }
    // A source thumbnail is a fallback, not necessarily an album cover.
    if song.cover_url.is_empty() {
        song.cover_url = value["thumbnail"]
            .as_str()
            .filter(|s| s.starts_with("https://"))
            .unwrap_or("")
            .into();
    }
    Some(song)
}

#[derive(Clone, Debug)]
pub struct Song {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub lyrics: String,
    pub cover_url: String,
}

pub fn cover(mgr: &Manager, song: &Song) -> Result<(String, Vec<u8>), String> {
    const LIMIT: u64 = 5 * 1024 * 1024;
    let _ = mgr;
    let response = client()
        .get(&song.cover_url)
        .call()
        .map_err(|e| format!("Pochette indisponible : {e}"))?;
    let mut bytes = Vec::new();
    response
        .into_body()
        .into_reader()
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > LIMIT {
        return Err("La pochette dépasse 5 Mo.".into());
    }
    let mime = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) && bytes.ends_with(&[0xff, 0xd9]) {
        "image/jpeg"
    } else {
        return Err("La source n’a pas fourni une pochette PNG ou JPEG valide.".into());
    };
    let (width, height) = image::ImageReader::new(std::io::Cursor::new(&bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?
        .into_dimensions()
        .map_err(|e| e.to_string())?;
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > 4_000_000 {
        return Err("Dimensions de pochette non prises en charge (maximum 4 mégapixels).".into());
    }
    image::load_from_memory(&bytes).map_err(|e| format!("Pochette illisible : {e}"))?;
    Ok((mime.into(), bytes))
}

/// Write to a same-directory copy, verify both frames, then replace the original.
/// A failed tag write never destroys the downloaded audio.
pub fn embed(path: &Path, song: &Song, mime: &str, cover: &[u8]) -> Result<(), String> {
    if path
        .extension()
        .is_none_or(|ext| !ext.eq_ignore_ascii_case("mp3"))
    {
        return Err("L’option dorée exige un fichier MP3.".into());
    }
    let temporary = path.with_extension("mp3.otm-tags.tmp");
    let result = (|| {
        std::fs::copy(path, &temporary).map_err(|e| e.to_string())?;
        let mut tag = match Tag::read_from_path(&temporary) {
            Ok(tag) => tag,
            Err(e) if matches!(e.kind, id3::ErrorKind::NoTag) => Tag::new(),
            Err(e) => return Err(e.to_string()),
        };
        tag.set_title(&song.title);
        tag.set_artist(&song.artist);
        if !song.album.is_empty() {
            tag.set_album(&song.album);
        }
        if !song.lyrics.is_empty() {
            tag.remove_all_lyrics();
            tag.add_frame(Lyrics {
                lang: "und".into(),
                description: String::new(),
                text: song.lyrics.clone(),
            });
        }
        if !cover.is_empty() {
            tag.remove_all_pictures();
            tag.add_frame(Picture {
                mime_type: mime.into(),
                picture_type: PictureType::CoverFront,
                description: String::new(),
                data: cover.to_vec(),
            });
        }
        tag.write_to_path(&temporary, Version::Id3v24)
            .map_err(|e| e.to_string())?;
        let verified = Tag::read_from_path(&temporary).map_err(|e| e.to_string())?;
        if (!song.lyrics.is_empty() && !verified.lyrics().any(|l| l.text == song.lyrics))
            || (!cover.is_empty() && !verified.pictures().any(|p| p.data == cover))
        {
            return Err("Échec de vérification des paroles ou de la pochette.".into());
        }
        std::fs::OpenOptions::new()
            .write(true)
            .open(&temporary)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())?;
        std::fs::rename(&temporary, path).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matching_rejects_other_versions_and_encodes_unicode() {
        let value = serde_json::json!({"title":"Artist - Song (Official Video)","duration":200});
        assert_eq!(identity(&value), Some(("Song".into(), "Artist".into())));
        let mut record =
            serde_json::json!({"trackName":"Song", "artistName":"Artist", "duration":200});
        assert!(matching_lyrics(&record, "Song", "Artist", 202.0));
        assert!(!matching_lyrics(&record, "Song", "Other", 200.0));
        assert!(!matching_lyrics(&record, "Song", "Artist", 220.0));
        record["instrumental"] = true.into();
        assert!(!matching_lyrics(&record, "Song", "Artist", 200.0));
        assert_eq!(query("أ &"), "%D8%A3%20%26");
    }

    #[test]
    #[ignore = "Calls the live LRCLIB and MusicBrainz APIs"]
    fn live_catalogue_returns_lyrics() {
        let metadata = serde_json::json!({"track":"Get Lucky", "artist":"Daft Punk", "album":"Random Access Memories", "duration":369});
        let song = resolve(&metadata).expect("song identity");
        assert!(!song.lyrics.is_empty(), "live lyrics unavailable");
    }

    #[test]
    #[ignore = "Downloads a real album cover from Cover Art Archive"]
    fn live_cover_is_decodable() {
        let song = Song {
            title: "Get Lucky".into(),
            artist: "Daft Punk".into(),
            album: "Random Access Memories".into(),
            lyrics: String::new(),
            cover_url:
                "https://coverartarchive.org/release/79215cdf-4764-4dee-b0b9-fec1643df7c5/front-500"
                    .into(),
        };
        let (_, image) = cover(&Manager::new(false), &song).expect("real cover");
        assert!(!image.is_empty());
    }
    #[test]
    fn mp3_tag_roundtrip_preserves_audio_and_unicode_lyrics() {
        let dir = std::env::temp_dir().join(format!("otm-gold-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sample.mp3");
        let audio = [0xff, 0xfb, 0x90, 0x64, 0, 0, 0, 0];
        std::fs::write(&path, audio).unwrap();
        let mut song = Song {
            title: "Test".into(),
            artist: "Artist".into(),
            album: "Album".into(),
            lyrics: "كلمات تجريبية\nTest lyrics".into(),
            cover_url: String::new(),
        };
        let picture = include_bytes!("../assets/otm-64.png");
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        song.cover_url = format!("http://{}/cover.png", server.server_addr().to_ip().unwrap());
        let serving = std::thread::spawn(move || {
            server
                .recv()
                .unwrap()
                .respond(tiny_http::Response::from_data(picture.to_vec()))
                .unwrap();
        });
        let (mime, downloaded_picture) = cover(&Manager::new(false), &song).unwrap();
        serving.join().unwrap();
        embed(&path, &song, &mime, &downloaded_picture).unwrap();
        let tag = Tag::read_from_path(&path).unwrap();
        assert_eq!(tag.title(), Some("Test"));
        assert_eq!(tag.lyrics().next().unwrap().text, song.lyrics);
        assert_eq!(tag.pictures().next().unwrap().data, picture);
        assert!(std::fs::read(&path).unwrap().ends_with(&audio));
        // A cover failure must still allow lyrics; existing artwork is retained.
        song.lyrics = "Updated lyrics".into();
        embed(&path, &song, "", &[]).unwrap();
        let tag = Tag::read_from_path(&path).unwrap();
        assert_eq!(tag.lyrics().next().unwrap().text, "Updated lyrics");
        assert_eq!(tag.pictures().next().unwrap().data, picture);
        // Likewise, cover-only updates preserve any existing lyrics.
        song.lyrics.clear();
        embed(&path, &song, &mime, &downloaded_picture).unwrap();
        assert_eq!(
            Tag::read_from_path(&path)
                .unwrap()
                .lyrics()
                .next()
                .unwrap()
                .text,
            "Updated lyrics"
        );
        assert!(std::fs::read(&path).unwrap().ends_with(&audio));
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(&dir).unwrap();
    }
}
