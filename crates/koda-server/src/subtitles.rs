//! Subtitles for the player: tracks inside the video file and subtitle files
//! next to it, all served as WebVTT. Image subtitles (PGS, VobSub) cannot be
//! turned into text and are listed as such.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde::Serialize;
use tokio::process::Command;
use utoipa::ToSchema;

use crate::streaming::Probe;

/// Text subtitle files picked up next to a video.
const TEXT_FILES: &[&str] = &["srt", "ass", "ssa", "vtt"];

#[derive(Debug, Clone, Serialize, ToSchema, PartialEq)]
pub struct Track {
    /// `e<N>` for the N-th track inside the file, `x<N>` for the N-th file next to it.
    pub id: String,
    /// ISO language code when known, e.g. `en`, `eng`, `rum`.
    pub language: Option<String>,
    pub label: String,
    pub codec: String,
    pub external: bool,
    pub default: bool,
    pub forced: bool,
    /// False for image subtitles: they are drawn into the picture instead,
    /// which needs converting the video.
    pub text: bool,
    /// Styled subtitles (ASS/SSA), which the player can show with their styling.
    pub styled: bool,
}

fn is_styled(codec: &str) -> bool {
    matches!(codec, "ass" | "ssa")
}

fn is_text_codec(codec: &str) -> bool {
    matches!(codec, "subrip" | "srt" | "ass" | "ssa" | "webvtt" | "vtt" | "mov_text" | "text")
}

/// Subtitle files next to `video` whose names start with the video's name,
/// sorted by name. `Film.en.forced.srt` -> language `en`, forced.
pub fn external(video: &Path) -> Vec<(PathBuf, Track)> {
    let (Some(dir), Some(stem)) = (video.parent(), video.file_stem().and_then(|s| s.to_str())) else {
        return vec![];
    };
    let Ok(entries) = std::fs::read_dir(dir) else { return vec![] };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let ext = p.extension().and_then(|x| x.to_str()).map(str::to_ascii_lowercase);
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            ext.is_some_and(|x| TEXT_FILES.contains(&x.as_str())) && name.starts_with(stem)
        })
        .collect();
    files.sort();
    files
        .into_iter()
        .enumerate()
        .map(|(n, p)| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("").to_owned();
            let ext = p.extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase();
            // tags between the video name and the extension
            let middle = name[stem.len()..name.len() - ext.len()].trim_matches('.').to_ascii_lowercase();
            let tags: Vec<&str> = middle.split(['.', '_', ' ', '-']).filter(|t| !t.is_empty()).collect();
            let language = tags
                .iter()
                .find(|t| {
                    (2..=3).contains(&t.len()) && t.bytes().all(|b| b.is_ascii_alphabetic()) && **t != "sdh"
                })
                .map(|t| (*t).to_owned());
            let forced = tags.contains(&"forced");
            let label = match (&language, forced) {
                (Some(l), true) => format!("{} (forced, file)", l.to_uppercase()),
                (Some(l), false) => format!("{} (file)", l.to_uppercase()),
                (None, _) => name.clone(),
            };
            let track = Track {
                id: format!("x{n}"),
                language,
                label,
                styled: is_styled(&ext),
                codec: ext,
                external: true,
                default: false,
                forced,
                text: true,
            };
            (p, track)
        })
        .collect()
}

/// Every subtitle track for a video: inside the file first, then files next to it.
pub fn tracks(video: &Path, probe: &Probe) -> Vec<Track> {
    let mut out: Vec<Track> = probe
        .subtitles
        .iter()
        .map(|s| {
            let lang = s.language.as_deref().map(str::to_uppercase);
            let mut label = match (&lang, &s.title) {
                (Some(l), Some(t)) => format!("{l} · {t}"),
                (Some(l), None) => l.clone(),
                (None, Some(t)) => t.clone(),
                (None, None) => format!("Track {}", s.index + 1),
            };
            if s.forced {
                label.push_str(" (forced)");
            }
            Track {
                id: format!("e{}", s.index),
                language: s.language.clone(),
                label,
                codec: s.codec.clone(),
                external: false,
                default: s.default,
                forced: s.forced,
                text: is_text_codec(&s.codec),
                styled: is_styled(&s.codec),
            }
        })
        .collect();
    out.extend(external(video).into_iter().map(|(_, t)| t));
    out
}

/// Converts one track to WebVTT with FFmpeg: `stream` is the subtitle index
/// inside `input`, or `None` when `input` is a subtitle file.
pub async fn to_vtt(input: &Path, stream: Option<usize>) -> anyhow::Result<String> {
    extract(input, stream, &["-f", "webvtt"]).await
}

/// One ASS/SSA track as is, styling included.
pub async fn to_ass(input: &Path, stream: Option<usize>) -> anyhow::Result<String> {
    extract(input, stream, &["-c:s", "copy", "-f", "ass"]).await
}

async fn extract(input: &Path, stream: Option<usize>, output: &[&str]) -> anyhow::Result<String> {
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-hide_banner", "-loglevel", "error", "-nostdin", "-i"]).arg(input);
    if let Some(i) = stream {
        cmd.args(["-map", &format!("0:s:{i}")]);
    }
    let out = cmd
        .args(output)
        .arg("-")
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|e| anyhow::anyhow!("could not run ffmpeg (is FFmpeg installed?): {e}"))?;
    if !out.status.success() {
        anyhow::bail!("could not read the subtitles: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The `n`-th attachment of `input` (a font, for MKV), as bytes.
pub async fn attachment(input: &Path, n: usize) -> anyhow::Result<Vec<u8>> {
    let stamp =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
    let out = std::env::temp_dir().join(format!("koda-attachment-{}-{n}-{stamp}", std::process::id()));
    // FFmpeg dumps the attachment, then complains that no output was given
    let run = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y"])
        .arg(format!("-dump_attachment:t:{n}"))
        .arg(&out)
        .arg("-i")
        .arg(input)
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|e| anyhow::anyhow!("could not run ffmpeg (is FFmpeg installed?): {e}"))?;
    let bytes = tokio::fs::read(&out).await;
    let _ = tokio::fs::remove_file(&out).await;
    match bytes {
        Ok(b) if !b.is_empty() => Ok(b),
        _ => anyhow::bail!("could not read the attachment: {}", String::from_utf8_lossy(&run.stderr).trim()),
    }
}

fn parse_time(t: &str) -> Option<f64> {
    let parts: Vec<&str> = t.trim().split(':').collect();
    let (h, m, s) = match parts.as_slice() {
        [h, m, s] => {
            (h.parse::<f64>().ok()?, m.parse::<f64>().ok()?, s.replace(',', ".").parse::<f64>().ok()?)
        }
        [m, s] => (0.0, m.parse::<f64>().ok()?, s.replace(',', ".").parse::<f64>().ok()?),
        _ => return None,
    };
    Some(h * 3600.0 + m * 60.0 + s)
}

fn format_time(t: f64) -> String {
    let ms = (t.max(0.0) * 1000.0).round() as u64;
    format!("{:02}:{:02}:{:02}.{:03}", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000)
}

/// Moves every cue `offset` seconds earlier, for a stream that starts
/// part-way into the film. Cues that end before the start are dropped.
pub fn shift_vtt(vtt: &str, offset: f64) -> String {
    if offset <= 0.0 {
        return vtt.to_owned();
    }
    let mut out = String::with_capacity(vtt.len());
    // blocks are separated by blank lines; the first one is the header
    for block in vtt.replace("\r\n", "\n").split("\n\n") {
        let mut lines = block.lines();
        let mut kept = Vec::new();
        let mut skip = false;
        for line in lines.by_ref() {
            if let Some((a, rest)) = line.split_once("-->") {
                let b = rest.split_whitespace().next().unwrap_or("");
                let settings = rest.trim_start().strip_prefix(b).unwrap_or("");
                match (parse_time(a), parse_time(b)) {
                    (Some(start), Some(end)) if end - offset > 0.0 => kept.push(format!(
                        "{} --> {}{settings}",
                        format_time(start - offset),
                        format_time(end - offset)
                    )),
                    (Some(_), Some(_)) => skip = true,
                    _ => kept.push(line.to_owned()),
                }
            } else {
                kept.push(line.to_owned());
            }
        }
        if !skip && !kept.is_empty() {
            out.push_str(&kept.join("\n"));
            out.push_str("\n\n");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cues_move_with_the_stream_start() {
        let vtt = "WEBVTT\n\n00:00:05.000 --> 00:00:07.000\nGone\n\n00:01:00.000 --> 00:01:02.500 line:90%\nHello\nthere\n";
        let s = shift_vtt(vtt, 30.0);
        assert!(s.starts_with("WEBVTT\n\n"));
        assert!(!s.contains("Gone"), "cue before the start is dropped");
        assert!(s.contains("00:00:30.000 --> 00:00:32.500 line:90%\nHello\nthere"), "{s}");
        assert_eq!(shift_vtt(vtt, 0.0), vtt);
        assert_eq!(parse_time("01:02:03,500"), Some(3723.5));
    }

    #[test]
    fn files_next_to_the_video_are_found() {
        let dir = std::env::temp_dir().join(format!("koda-subs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let video = dir.join("Film (2020) - WEB-DL 1080p.mkv");
        for f in [
            "Film (2020) - WEB-DL 1080p.en.srt",
            "Film (2020) - WEB-DL 1080p.ro.forced.ass",
            "Other.en.srt",
            "Film (2020) - WEB-DL 1080p.idx",
        ] {
            std::fs::write(dir.join(f), "x").unwrap();
        }
        let found = external(&video);
        std::fs::remove_dir_all(&dir).unwrap();
        let t: Vec<_> = found.iter().map(|(_, t)| (t.id.as_str(), t.language.as_deref(), t.forced)).collect();
        assert_eq!(t, [("x0", Some("en"), false), ("x1", Some("ro"), true)]);
    }

    #[test]
    fn image_subtitles_are_marked() {
        let probe = Probe {
            subtitles: vec![
                crate::streaming::SubtitleStream {
                    index: 0,
                    codec: "subrip".into(),
                    language: Some("eng".into()),
                    ..Default::default()
                },
                crate::streaming::SubtitleStream {
                    index: 1,
                    codec: "hdmv_pgs_subtitle".into(),
                    forced: true,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let t = tracks(Path::new("/nowhere/x.mkv"), &probe);
        assert_eq!((t[0].label.as_str(), t[0].text), ("ENG", true));
        assert_eq!((t[1].label.as_str(), t[1].text), ("Track 2 (forced)", false));
    }
}

/// Needs FFmpeg on PATH: `cargo test -p koda-server -- --ignored ffmpeg`.
#[cfg(test)]
mod ffmpeg_tests {
    use super::*;

    #[tokio::test]
    #[ignore]
    async fn ffmpeg_subtitles_become_vtt() {
        let dir = std::env::temp_dir().join(format!("koda-ffmpeg-subs-{}", std::process::id()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let srt = dir.join("Film.en.srt");
        tokio::fs::write(
            &srt,
            "1\n00:00:01,000 --> 00:00:02,500\nHello\n\n2\n00:00:04,000 --> 00:00:05,000\nWorld\n",
        )
        .await
        .unwrap();
        let mkv = dir.join("Film.mkv");
        let ok = Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc=duration=6:size=320x180:rate=24",
                "-i",
            ])
            .arg(&srt)
            .args(["-c:v", "libx264", "-c:s", "srt", "-metadata:s:s:0", "language=eng"])
            .arg(&mkv)
            .status()
            .await
            .unwrap();
        assert!(ok.success(), "could not create test clip");

        let probe = crate::streaming::probe(&mkv).await.unwrap();
        let t = tracks(&mkv, &probe);
        assert_eq!(t.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["e0", "x0"]);
        let inside = to_vtt(&mkv, Some(0)).await.unwrap();
        assert!(inside.starts_with("WEBVTT") && inside.contains("Hello"), "{inside}");
        let file = to_vtt(&srt, None).await.unwrap();
        assert!(shift_vtt(&file, 3.0).contains("00:00:01.000 --> 00:00:02.000\nWorld"), "{file}");

        // a styled track inside the film; FFmpeg cannot make image subtitles from text,
        // so drawing those in is covered by the argument test only
        let styled_mkv = dir.join("Styled.mkv");
        let ok = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i"])
            .arg("testsrc=duration=6:size=320x180:rate=24")
            .arg("-i")
            .arg(&srt)
            .args(["-c:v", "libx264", "-c:s", "ass"])
            .arg(&styled_mkv)
            .status()
            .await
            .unwrap();
        assert!(ok.success(), "could not create styled clip");
        let probe = crate::streaming::probe(&styled_mkv).await.unwrap();
        let t = tracks(&styled_mkv, &probe);
        assert_eq!((t[0].styled, t[0].text), (true, true));
        let styled = to_ass(&styled_mkv, Some(0)).await.unwrap();
        assert!(styled.contains("[Script Info]") && styled.contains("Hello"), "{styled}");

        // a font attached to the film comes back byte for byte
        let font = dir.join("Fancy.ttf");
        tokio::fs::write(&font, b"not really a font").await.unwrap();
        let with_font = dir.join("Font.mkv");
        let ok = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(&styled_mkv)
            .arg("-attach")
            .arg(&font)
            .args(["-metadata:s:t", "mimetype=application/x-truetype-font", "-c", "copy"])
            .arg(&with_font)
            .status()
            .await
            .unwrap();
        assert!(ok.success(), "could not attach a font");
        let probe = crate::streaming::probe(&with_font).await.unwrap();
        let fonts = probe.fonts.unwrap();
        assert_eq!((fonts.len(), fonts[0].name.as_str()), (1, "Fancy.ttf"));
        assert_eq!(attachment(&with_font, 0).await.unwrap(), b"not really a font");
        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }
}
