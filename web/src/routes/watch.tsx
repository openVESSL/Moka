import { useRouter } from "@tanstack/react-router";
import Hls from "hls.js";
import { ArrowLeft, Maximize, Pause, Play } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { api, unwrap, type Schemas } from "../api/client";

type PlayResponse = Schemas["PlayResponse"];

const SAVE_EVERY_MS = 10_000;

/** What this browser can decode, asked the way the player will use it (Media Source). */
function detectCaps(): Schemas["ClientCaps"] {
  const ms = window.MediaSource as typeof MediaSource | undefined;
  const can = (mime: string) => !!ms?.isTypeSupported?.(mime);
  const el = document.createElement("video");
  const plays = (mime: string) => el.canPlayType(mime) !== "";
  const pick = (table: Record<string, string>) => Object.keys(table).filter((k) => can(table[k]));
  return {
    video_codecs: pick({
      h264: 'video/mp4; codecs="avc1.640028"',
      hevc: 'video/mp4; codecs="hvc1.1.6.L120.90"',
      av1: 'video/mp4; codecs="av01.0.08M.08"',
      vp9: 'video/webm; codecs="vp9"',
    }),
    audio_codecs: pick({
      aac: 'audio/mp4; codecs="mp4a.40.2"',
      mp3: "audio/mpeg",
      opus: 'audio/webm; codecs="opus"',
      flac: 'audio/mp4; codecs="flac"',
      ac3: 'audio/mp4; codecs="ac-3"',
      eac3: 'audio/mp4; codecs="ec-3"',
    }),
    containers: (["mp4", "webm", "mkv"] as const).filter((c) =>
      plays(c === "mkv" ? "video/x-matroska" : `video/${c}`),
    ),
    max_height: null,
  };
}

function clock(s: number) {
  if (!Number.isFinite(s)) return "0:00";
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = Math.floor(s % 60);
  return h ? `${h}:${String(m).padStart(2, "0")}:${String(sec).padStart(2, "0")}` : `${m}:${String(sec).padStart(2, "0")}`;
}

/**
 * Plays a file directly when the browser can, otherwise as HLS made on the
 * fly. The stream may start part-way into the film, so times shown are
 * `offset + currentTime`; seeking outside what is ready restarts the stream there.
 */
export function WatchPage({ fileId }: { fileId: number }) {
  const router = useRouter();
  const video = useRef<HTMLVideoElement>(null);
  const hls = useRef<Hls | null>(null);
  const session = useRef<string | null>(null);
  const lastSave = useRef(0);
  const [stream, setStream] = useState<PlayResponse>();
  const [error, setError] = useState<string>();
  const [now, setNow] = useState(0);
  const [paused, setPaused] = useState(true);
  const [loading, setLoading] = useState(true);

  const offset = stream?.start_seconds ?? 0;
  const duration = stream?.duration_seconds ?? 0;

  const stopSession = useCallback(() => {
    hls.current?.destroy();
    hls.current = null;
    if (session.current) {
      api.DELETE("/api/v1/hls/{session}", { params: { path: { session: session.current } } });
      session.current = null;
    }
  }, []);

  const open = useCallback(
    async (start: number) => {
      setLoading(true);
      setError(undefined);
      stopSession();
      try {
        const res = await unwrap(
          api.POST("/api/v1/files/{id}/play", {
            params: { path: { id: fileId } },
            body: { caps: detectCaps(), start_seconds: start },
          }),
        );
        session.current = res.session ?? null;
        setStream(res);
        const v = video.current!;
        if (res.method === "direct") {
          v.src = res.url;
          v.currentTime = start;
        } else if (Hls.isSupported()) {
          const h = new Hls({ maxBufferLength: 30 });
          h.loadSource(res.url);
          h.attachMedia(v);
          h.on(Hls.Events.ERROR, (_e, d) => d.fatal && setError("The stream stopped. Try again."));
          hls.current = h;
        } else {
          v.src = res.url; // Safari plays HLS natively
        }
        await v.play().catch(() => undefined);
      } catch (e) {
        setError(e instanceof Error ? e.message : "Could not start playback.");
      } finally {
        setLoading(false);
      }
    },
    [fileId, stopSession],
  );

  useEffect(() => {
    let cancelled = false;
    api.GET("/api/v1/files/{id}/progress", { params: { path: { id: fileId } } }).then(({ data }) => {
      if (cancelled) return;
      const resume = data && !data.played && data.position_seconds > 30 ? data.position_seconds : 0;
      open(resume);
    });
    return () => {
      cancelled = true;
      stopSession();
    };
  }, [fileId, open, stopSession]);

  function save(force = false) {
    const v = video.current;
    if (!v || !duration) return;
    const t = Date.now();
    if (!force && t - lastSave.current < SAVE_EVERY_MS) return;
    lastSave.current = t;
    const position = offset + v.currentTime;
    // the last stretch is credits: count the film as watched
    const played = position / duration > 0.92;
    api.PUT("/api/v1/files/{id}/progress", {
      params: { path: { id: fileId } },
      body: { position_seconds: played ? 0 : position, played },
    });
  }

  function seek(target: number) {
    const v = video.current;
    if (!v || !stream) return;
    if (stream.method === "direct") {
      v.currentTime = target;
      return;
    }
    const local = target - offset;
    const end = v.buffered.length ? v.buffered.end(v.buffered.length - 1) : 0;
    if (local >= 0 && local <= end) v.currentTime = local;
    else open(target);
  }

  return (
    <main className="fixed inset-0 z-50 bg-black text-chalk">
      <video
        ref={video}
        playsInline
        className="size-full bg-black"
        onClick={() => (video.current?.paused ? video.current.play() : video.current?.pause())}
        onTimeUpdate={(e) => {
          setNow(offset + e.currentTarget.currentTime);
          save();
        }}
        onPlay={() => setPaused(false)}
        onPause={() => {
          setPaused(true);
          save(true);
        }}
        onEnded={() => save(true)}
        onWaiting={() => setLoading(true)}
        onPlaying={() => setLoading(false)}
        onError={() => stream?.method === "direct" && setError("This file could not be played.")}
      />

      <button
        type="button"
        onClick={() => {
          save(true);
          router.history.back();
        }}
        className="absolute top-4 left-4 inline-flex items-center gap-2 rounded bg-black/50 px-3 py-2 text-sm hover:bg-black/80"
      >
        <ArrowLeft aria-hidden className="size-4" /> Back
      </button>

      {stream && stream.method !== "direct" && (
        <p className="absolute top-5 right-5 max-w-sm text-right text-xs text-chalk-dim">
          {stream.method === "remux" ? "Repackaging for this browser" : "Converting for this browser"}:{" "}
          {stream.reasons.join(", ")}
        </p>
      )}
      {loading && !error && (
        <div aria-live="polite" className="pointer-events-none absolute inset-0 grid place-items-center text-chalk-dim">
          Loading…
        </div>
      )}
      {error && (
        <div role="alert" className="absolute inset-0 grid place-items-center">
          <div className="text-center">
            <p className="text-lg">{error}</p>
            <button type="button" className="btn-chalk mt-4" onClick={() => open(now)}>
              Try again
            </button>
          </div>
        </div>
      )}

      <div className="absolute inset-x-0 bottom-0 flex items-center gap-4 bg-gradient-to-t from-black/80 to-transparent px-5 pt-10 pb-5">
        <button
          type="button"
          aria-label={paused ? "Play" : "Pause"}
          onClick={() => (paused ? video.current?.play() : video.current?.pause())}
        >
          {paused ? <Play aria-hidden className="size-6" fill="currentColor" /> : <Pause aria-hidden className="size-6" fill="currentColor" />}
        </button>
        <span className="data w-16 text-right text-sm">{clock(now)}</span>
        <input
          type="range"
          aria-label="Position"
          min={0}
          max={Math.max(duration, 1)}
          step={1}
          value={Math.min(now, duration)}
          onChange={(e) => {
            const t = Number(e.target.value);
            setNow(t);
            seek(t);
          }}
          className="flex-1 accent-grease"
        />
        <span className="data w-16 text-sm text-chalk-dim">{clock(duration)}</span>
        <button type="button" aria-label="Full screen" onClick={() => document.documentElement.requestFullscreen?.()}>
          <Maximize aria-hidden className="size-5" />
        </button>
      </div>
    </main>
  );
}
