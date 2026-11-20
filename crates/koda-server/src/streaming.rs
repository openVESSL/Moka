//! Playback: decide between sending the file as is, repackaging it (remux)
//! or converting it (transcode), and run FFmpeg for the last two.
//!
//! Remux and transcode produce HLS (fragmented MP4) in a per-session folder.
//! A session starts at a given time; seeking outside what has been produced
//! starts a new session at the new time, so any point is reachable quickly.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::process::{Child, Command};
use utoipa::ToSchema;

/// What FFprobe says about a file.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Probe {
    /// FFmpeg format names, e.g. "matroska,webm" or "mov,mp4,m4a,3gp,3g2,mj2".
    pub format: String,
    pub duration_seconds: f64,
    pub video: Option<VideoStream>,
    pub audio: Vec<AudioStream>,
    pub subtitles: Vec<SubtitleStream>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct VideoStream {
    pub codec: String,
    pub profile: Option<String>,
    pub width: u32,
    pub height: u32,
    pub pix_fmt: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AudioStream {
    /// Stream index among audio streams (for `-map 0:a:N`).
    pub index: usize,
    pub codec: String,
    pub channels: u32,
    pub language: Option<String>,
    pub default: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SubtitleStream {
    pub index: usize,
    pub codec: String,
    pub language: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub default: bool,
    #[serde(default)]
    pub forced: bool,
}

pub async fn probe(path: &Path) -> anyhow::Result<Probe> {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams"])
        .arg(path)
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|e| anyhow::anyhow!("could not run ffprobe (is FFmpeg installed?): {e}"))?;
    if !out.status.success() {
        anyhow::bail!("ffprobe could not read the file: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    parse_probe(&String::from_utf8_lossy(&out.stdout))
}

pub fn parse_probe(json: &str) -> anyhow::Result<Probe> {
    let v: serde_json::Value = serde_json::from_str(json)?;
    let s = |x: &serde_json::Value, k: &str| x.get(k).and_then(|y| y.as_str()).map(str::to_owned);
    let mut p = Probe {
        format: s(&v["format"], "format_name").unwrap_or_default(),
        duration_seconds: s(&v["format"], "duration").and_then(|d| d.parse().ok()).unwrap_or(0.0),
        ..Default::default()
    };
    let streams = v["streams"].as_array().cloned().unwrap_or_default();
    for st in &streams {
        let codec = s(st, "codec_name").unwrap_or_default();
        let lang = st["tags"]["language"].as_str().map(str::to_owned).filter(|l| l != "und");
        match st["codec_type"].as_str() {
            // attached cover pictures are "video" too
            Some("video") if p.video.is_none() && st["disposition"]["attached_pic"].as_i64() != Some(1) => {
                p.video = Some(VideoStream {
                    codec,
                    profile: s(st, "profile"),
                    width: st["width"].as_u64().unwrap_or(0) as u32,
                    height: st["height"].as_u64().unwrap_or(0) as u32,
                    pix_fmt: s(st, "pix_fmt"),
                });
            }
            Some("audio") => p.audio.push(AudioStream {
                index: p.audio.len(),
                codec,
                channels: st["channels"].as_u64().unwrap_or(2) as u32,
                language: lang,
                default: st["disposition"]["default"].as_i64() == Some(1),
            }),
            Some("subtitle") => p.subtitles.push(SubtitleStream {
                index: p.subtitles.len(),
                codec,
                language: lang,
                title: s(&st["tags"], "title"),
                default: st["disposition"]["default"].as_i64() == Some(1),
                forced: st["disposition"]["forced"].as_i64() == Some(1),
            }),
            _ => {}
        }
    }
    Ok(p)
}

/// What the browser said it can decode (from `MediaSource.isTypeSupported`).
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct ClientCaps {
    /// e.g. ["h264", "hevc", "av1", "vp9"]
    pub video_codecs: Vec<String>,
    /// e.g. ["aac", "mp3", "opus", "flac", "ac3", "eac3"]
    pub audio_codecs: Vec<String>,
    /// Containers the browser plays as a plain file, e.g. ["mp4", "webm"]
    pub containers: Vec<String>,
    /// Highest video height the device wants; larger is scaled down when transcoding.
    pub max_height: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    Direct,
    Remux,
    Transcode,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Decision {
    pub method: Method,
    pub copy_video: bool,
    pub copy_audio: bool,
    pub audio_index: Option<usize>,
    /// Plain-language reasons the file is not sent as is.
    pub reasons: Vec<String>,
}

fn container_of(format: &str) -> &'static str {
    let f = format.to_ascii_lowercase();
    if f.contains("mp4") || f.contains("mov") {
        "mp4"
    } else if f.contains("webm") && !f.contains("matroska") {
        "webm"
    } else if f.contains("matroska") {
        "mkv"
    } else if f.contains("avi") {
        "avi"
    } else if f.contains("mpegts") {
        "ts"
    } else {
        "other"
    }
}

/// Picks the cheapest way to play `p` on a client with `caps`.
pub fn decide(p: &Probe, caps: &ClientCaps) -> Decision {
    let has = |list: &[String], x: &str| list.iter().any(|c| c.eq_ignore_ascii_case(x));
    // first default audio track, else the first one
    let audio = p.audio.iter().find(|a| a.default).or_else(|| p.audio.first());
    let mut reasons = Vec::new();

    let video_ok = match &p.video {
        Some(v) => {
            let ten_bit_h264 = v.codec == "h264" && v.pix_fmt.as_deref().is_some_and(|f| f.contains("10"));
            let ok = has(&caps.video_codecs, &v.codec) && !ten_bit_h264;
            if !ok {
                reasons.push(format!("video codec {} is not supported here", v.codec.to_uppercase()));
            }
            ok
        }
        None => true,
    };
    let too_tall = matches!((&p.video, caps.max_height), (Some(v), Some(m)) if v.height > m);
    if too_tall {
        reasons.push("video is larger than this device plays".into());
    }
    let audio_ok = audio.is_none_or(|a| has(&caps.audio_codecs, &a.codec));
    if let Some(a) = audio.filter(|_| !audio_ok) {
        reasons.push(format!("audio codec {} is not supported here", a.codec.to_uppercase()));
    }
    let container = container_of(&p.format);
    let container_ok = has(&caps.containers, container);
    if !container_ok {
        reasons.push(format!("{} files do not play directly in this browser", container.to_uppercase()));
    }

    let method = if video_ok && !too_tall && audio_ok && container_ok {
        Method::Direct
    } else if video_ok && !too_tall {
        Method::Remux
    } else {
        Method::Transcode
    };
    Decision {
        method,
        copy_video: method != Method::Transcode,
        copy_audio: audio_ok,
        audio_index: audio.map(|a| a.index),
        reasons,
    }
}

/// Video encoder used when a film must be converted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Encoder {
    /// On the processor (libx264). Works everywhere.
    #[default]
    Software,
    /// NVIDIA graphics cards.
    Nvenc,
    /// Intel and AMD graphics on Linux.
    Vaapi,
    /// Intel Quick Sync.
    Qsv,
}

/// Hardware video conversion, stored under the `playback` setting.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct HwAccel {
    #[serde(default)]
    pub encoder: Encoder,
    /// Render device for VAAPI, e.g. `/dev/dri/renderD128`.
    #[serde(default)]
    pub device: String,
}

impl HwAccel {
    pub const KEY: &'static str = "playback";
}

/// FFmpeg arguments for an HLS session starting at `start` seconds.
pub fn ffmpeg_args(input: &Path, d: &Decision, start: f64, max_height: u32, hw: &HwAccel) -> Vec<String> {
    let mut a: Vec<String> =
        vec!["-hide_banner".into(), "-loglevel".into(), "error".into(), "-nostdin".into()];
    let encoder = if d.copy_video { Encoder::Software } else { hw.encoder };
    if encoder == Encoder::Vaapi {
        let dev = if hw.device.is_empty() { "/dev/dri/renderD128" } else { hw.device.as_str() };
        a.extend(["-vaapi_device".into(), dev.into()]);
    }
    if start > 0.0 {
        a.extend(["-ss".into(), format!("{start:.3}")]);
    }
    a.extend(["-i".into(), input.to_string_lossy().into_owned()]);
    a.extend(["-map".into(), "0:v:0".into()]);
    if let Some(i) = d.audio_index {
        a.extend(["-map".into(), format!("0:a:{i}")]);
    }
    // same keyframe spacing for every encoder: 4-second segments at 24 fps
    let gop = ["-g", "96", "-keyint_min", "96"].map(String::from);
    let scale = format!("scale=-2:'min({max_height},ih)'");
    if d.copy_video {
        a.extend(["-c:v".into(), "copy".into()]);
    } else if encoder == Encoder::Nvenc {
        a.extend(
            ["-c:v", "h264_nvenc", "-preset", "p4", "-rc", "vbr", "-cq", "23", "-profile:v", "high"]
                .map(String::from),
        );
        a.extend(gop);
        a.extend(["-vf".into(), format!("{scale},format=yuv420p")]);
    } else if encoder == Encoder::Vaapi {
        a.extend(["-c:v", "h264_vaapi", "-qp", "23"].map(String::from));
        a.extend(gop);
        a.extend(["-vf".into(), format!("{scale},format=nv12,hwupload")]);
    } else if encoder == Encoder::Qsv {
        a.extend(["-c:v", "h264_qsv", "-global_quality", "23", "-profile:v", "high"].map(String::from));
        a.extend(gop);
        a.extend(["-vf".into(), format!("{scale},format=nv12")]);
    } else {
        a.extend(
            [
                "-c:v",
                "libx264",
                "-preset",
                "veryfast",
                "-crf",
                "22",
                "-pix_fmt",
                "yuv420p",
                "-profile:v",
                "high",
                "-g",
                "96",
                "-keyint_min",
                "96",
                "-sc_threshold",
                "0",
            ]
            .map(String::from),
        );
        a.extend(["-vf".into(), scale]);
    }
    if d.audio_index.is_some() {
        if d.copy_audio {
            a.extend(["-c:a".into(), "copy".into()]);
        } else {
            a.extend(["-c:a", "aac", "-ac", "2", "-b:a", "192k"].map(String::from));
        }
    }
    a.extend(
        [
            "-sn",
            "-dn",
            "-map_metadata",
            "-1",
            "-f",
            "hls",
            "-hls_time",
            "4",
            "-hls_list_size",
            "0",
            "-hls_playlist_type",
            "event",
            "-hls_segment_type",
            "fmp4",
            "-hls_fmp4_init_filename",
            "init.mp4",
            "-hls_segment_filename",
            "seg_%05d.m4s",
            "index.m3u8",
        ]
        .map(String::from),
    );
    a
}

pub struct Session {
    pub dir: PathBuf,
    child: Child,
    pub last_access: Instant,
}

/// Running HLS sessions. Idle ones are stopped and their files removed.
#[derive(Clone, Default)]
pub struct Sessions(Arc<Mutex<HashMap<String, Session>>>);

pub const IDLE_TIMEOUT: Duration = Duration::from_secs(90);

impl Sessions {
    pub async fn start(
        &self,
        root: &Path,
        input: &Path,
        d: &Decision,
        start: f64,
        max_height: u32,
        hw: &HwAccel,
    ) -> anyhow::Result<String> {
        let mut id_bytes = [0u8; 12];
        getrandom::fill(&mut id_bytes).map_err(|e| anyhow::anyhow!("rng: {e}"))?;
        let id: String = id_bytes.iter().map(|b| format!("{b:02x}")).collect();
        let dir = root.join(&id);
        tokio::fs::create_dir_all(&dir).await?;
        let child = Command::new("ffmpeg")
            .args(ffmpeg_args(input, d, start, max_height, hw))
            .current_dir(&dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| anyhow::anyhow!("could not start ffmpeg (is FFmpeg installed?): {e}"))?;
        self.0
            .lock()
            .expect("sessions lock")
            .insert(id.clone(), Session { dir: dir.clone(), child, last_access: Instant::now() });

        // the player needs the playlist and a first segment before it can start
        let playlist = dir.join("index.m3u8");
        for _ in 0..150 {
            if let Ok(text) = tokio::fs::read_to_string(&playlist).await
                && text.contains(".m4s")
            {
                return Ok(id);
            }
            if let Some(err) = self.exited_with_error(&id).await {
                self.stop(&id).await;
                anyhow::bail!("ffmpeg failed: {err}");
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        self.stop(&id).await;
        anyhow::bail!("ffmpeg did not produce video in time")
    }

    async fn exited_with_error(&self, id: &str) -> Option<String> {
        let child = {
            let mut map = self.0.lock().expect("sessions lock");
            let s = map.get_mut(id)?;
            match s.child.try_wait() {
                Ok(Some(status)) if !status.success() => s.child.stderr.take(),
                _ => return None,
            }
        };
        let mut msg = String::new();
        if let Some(mut stderr) = child {
            use tokio::io::AsyncReadExt;
            let _ = stderr.read_to_string(&mut msg).await;
        }
        Some(msg.trim().lines().last().unwrap_or("unknown error").to_owned())
    }

    /// Folder of a session, marking it as used.
    pub fn touch(&self, id: &str) -> Option<PathBuf> {
        let mut map = self.0.lock().expect("sessions lock");
        let s = map.get_mut(id)?;
        s.last_access = Instant::now();
        Some(s.dir.clone())
    }

    pub async fn stop(&self, id: &str) {
        let removed = self.0.lock().expect("sessions lock").remove(id);
        if let Some(mut s) = removed {
            let _ = s.child.kill().await;
            let _ = tokio::fs::remove_dir_all(&s.dir).await;
        }
    }

    /// Stops sessions nobody asked for in a while. Runs forever.
    pub async fn reap(self) {
        loop {
            tokio::time::sleep(Duration::from_secs(30)).await;
            let idle: Vec<String> = self
                .0
                .lock()
                .expect("sessions lock")
                .iter()
                .filter(|(_, s)| s.last_access.elapsed() > IDLE_TIMEOUT)
                .map(|(k, _)| k.clone())
                .collect();
            for id in idle {
                self.stop(&id).await;
            }
        }
    }
}

/// HLS file names a session may serve. Anything else is refused.
pub fn valid_hls_name(name: &str) -> bool {
    name == "index.m3u8"
        || name == "init.mp4"
        || name
            .strip_prefix("seg_")
            .and_then(|r| r.strip_suffix(".m4s"))
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MKV_HEVC_DTS: &str = r#"{
      "streams": [
        {"codec_type":"video","codec_name":"hevc","profile":"Main 10","width":3840,"height":2160,"pix_fmt":"yuv420p10le","disposition":{"attached_pic":0}},
        {"codec_type":"audio","codec_name":"dts","channels":6,"tags":{"language":"eng"},"disposition":{"default":1}},
        {"codec_type":"audio","codec_name":"aac","channels":2,"tags":{"language":"fre"},"disposition":{"default":0}},
        {"codec_type":"subtitle","codec_name":"subrip","tags":{"language":"rum"}},
        {"codec_type":"video","codec_name":"mjpeg","disposition":{"attached_pic":1}}
      ],
      "format": {"format_name":"matroska,webm","duration":"7260.5"}
    }"#;

    fn chrome() -> ClientCaps {
        ClientCaps {
            video_codecs: vec!["h264".into(), "vp9".into(), "av1".into()],
            audio_codecs: vec!["aac".into(), "mp3".into(), "opus".into(), "flac".into()],
            containers: vec!["mp4".into(), "webm".into()],
            max_height: None,
        }
    }

    #[test]
    fn probe_output_is_read() {
        let p = parse_probe(MKV_HEVC_DTS).unwrap();
        assert_eq!(p.duration_seconds, 7260.5);
        let v = p.video.as_ref().unwrap();
        assert_eq!((v.codec.as_str(), v.height), ("hevc", 2160), "cover art is not the video");
        assert_eq!(p.audio.len(), 2);
        assert!(p.audio[0].default);
        assert_eq!(p.subtitles[0].language.as_deref(), Some("rum"));
    }

    #[test]
    fn decisions() {
        let p = parse_probe(MKV_HEVC_DTS).unwrap();
        let d = decide(&p, &chrome());
        assert_eq!(d.method, Method::Transcode);
        assert!(!d.copy_audio);
        assert_eq!(d.audio_index, Some(0), "default track is used");
        assert_eq!(d.reasons.len(), 3, "{:?}", d.reasons);

        let mut caps = chrome();
        caps.video_codecs.push("hevc".into());
        assert_eq!(decide(&p, &caps).method, Method::Remux, "only audio and container need changing");

        let mp4 = Probe {
            format: "mov,mp4,m4a,3gp,3g2,mj2".into(),
            video: Some(VideoStream {
                codec: "h264".into(),
                height: 1080,
                pix_fmt: Some("yuv420p".into()),
                ..Default::default()
            }),
            audio: vec![AudioStream { codec: "aac".into(), channels: 2, ..Default::default() }],
            ..Default::default()
        };
        let d = decide(&mp4, &chrome());
        assert_eq!((d.method, d.reasons.len()), (Method::Direct, 0));

        let mut small = chrome();
        small.max_height = Some(720);
        assert_eq!(decide(&mp4, &small).method, Method::Transcode);
    }

    #[test]
    fn ffmpeg_arguments() {
        let p = parse_probe(MKV_HEVC_DTS).unwrap();
        let d = decide(&p, &chrome());
        let args = ffmpeg_args(Path::new("/m/film.mkv"), &d, 600.0, 1080, &HwAccel::default()).join(" ");
        assert!(args.starts_with("-hide_banner -loglevel error -nostdin -ss 600.000 -i /m/film.mkv"));
        assert!(args.contains("-c:v libx264"));
        assert!(args.contains("-c:a aac -ac 2"));
        assert!(args.contains("-hls_segment_type fmp4"));

        let mut caps = chrome();
        caps.video_codecs.push("hevc".into());
        let args =
            ffmpeg_args(Path::new("f.mkv"), &decide(&p, &caps), 0.0, 1080, &HwAccel::default()).join(" ");
        assert!(args.contains("-c:v copy") && !args.contains("-ss"));

        let hw = |encoder| HwAccel { encoder, device: String::new() };
        let a = ffmpeg_args(Path::new("f.mkv"), &d, 0.0, 720, &hw(Encoder::Vaapi)).join(" ");
        assert!(a.starts_with("-hide_banner -loglevel error -nostdin -vaapi_device /dev/dri/renderD128 -i"));
        assert!(a.contains("-c:v h264_vaapi") && a.contains("format=nv12,hwupload"), "{a}");
        let a = ffmpeg_args(Path::new("f.mkv"), &d, 0.0, 720, &hw(Encoder::Nvenc)).join(" ");
        assert!(a.contains("-c:v h264_nvenc") && !a.contains("libx264"), "{a}");
        // nothing to convert: the encoder is not touched
        let copy = decide(&p, &caps);
        let a = ffmpeg_args(Path::new("f.mkv"), &copy, 0.0, 720, &hw(Encoder::Qsv)).join(" ");
        assert!(!a.contains("qsv") && a.contains("-c:v copy"));
    }

    #[test]
    fn only_session_files_are_served() {
        assert!(valid_hls_name("index.m3u8"));
        assert!(valid_hls_name("init.mp4"));
        assert!(valid_hls_name("seg_00012.m4s"));
        assert!(!valid_hls_name("seg_.m4s"));
        assert!(!valid_hls_name("../koda.db"));
        assert!(!valid_hls_name("seg_1.m4s/../../x"));
    }
}

/// Needs FFmpeg on PATH: `cargo test -p koda-server -- --ignored ffmpeg`.
#[cfg(test)]
mod ffmpeg_tests {
    use super::*;

    async fn make_clip(dir: &Path, name: &str, vcodec: &str, acodec: &str) -> PathBuf {
        let out = dir.join(name);
        let status = Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc=duration=12:size=640x360:rate=24",
            ])
            .args([
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=12",
                "-c:v",
                vcodec,
                "-c:a",
                acodec,
                "-shortest",
            ])
            .arg(&out)
            .status()
            .await
            .unwrap();
        assert!(status.success(), "could not create test clip");
        out
    }

    #[tokio::test]
    #[ignore]
    async fn ffmpeg_remux_and_transcode_produce_hls() {
        let dir = std::env::temp_dir().join(format!("koda-ffmpeg-{}", std::process::id()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let caps = ClientCaps {
            video_codecs: vec!["h264".into()],
            audio_codecs: vec!["aac".into()],
            containers: vec!["mp4".into()],
            max_height: None,
        };
        let sessions = Sessions::default();

        // H.264 + AC-3 in MKV: video is fine, container and audio are not
        let mkv = make_clip(&dir, "remux.mkv", "libx264", "ac3").await;
        let p = probe(&mkv).await.unwrap();
        assert!((p.duration_seconds - 12.0).abs() < 0.5);
        let d = decide(&p, &caps);
        assert_eq!(d.method, Method::Remux);
        let id = sessions.start(&dir.join("t"), &mkv, &d, 0.0, 1080, &HwAccel::default()).await.unwrap();
        let playlist =
            tokio::fs::read_to_string(sessions.touch(&id).unwrap().join("index.m3u8")).await.unwrap();
        assert!(playlist.contains("#EXT-X-MAP:URI=\"init.mp4\""), "{playlist}");
        sessions.stop(&id).await;

        // HEVC: the video itself must be converted; start part-way in
        let hevc = make_clip(&dir, "hevc.mkv", "libx265", "aac").await;
        let d = decide(&probe(&hevc).await.unwrap(), &caps);
        assert_eq!(d.method, Method::Transcode);
        let id = sessions.start(&dir.join("t"), &hevc, &d, 6.0, 720, &HwAccel::default()).await.unwrap();
        let seg_dir = sessions.touch(&id).unwrap();
        assert!(seg_dir.join("init.mp4").exists());
        sessions.stop(&id).await;
        assert!(!seg_dir.exists(), "stopping removes the session's files");
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}
