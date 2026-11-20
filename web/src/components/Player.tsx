import { Disc3, Pause, Play, SkipBack, SkipForward, X } from "lucide-react";
import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { cover } from "../api/client";

export type QueueItem = {
  fileId: number;
  title: string;
  artist: string;
  album: string;
  /** Album MusicBrainz id, for the cover. */
  mbid?: string | null;
};

type Player = {
  current?: QueueItem;
  playing: boolean;
  /** Replaces the queue and starts at `index`. */
  play: (queue: QueueItem[], index: number) => void;
  toggle: () => void;
};

const PlayerContext = createContext<Player | null>(null);

export function usePlayer() {
  const p = useContext(PlayerContext);
  if (!p) throw new Error("usePlayer outside PlayerProvider");
  return p;
}

const streamUrl = (fileId: number) => `/api/v1/files/${fileId}/stream`;

function clock(s: number) {
  if (!Number.isFinite(s) || s < 0) return "0:00";
  const m = Math.floor(s / 60);
  return `${m}:${String(Math.floor(s % 60)).padStart(2, "0")}`;
}

/** Holds the music queue and one <audio> element shared by every page. */
export function PlayerProvider({ children }: { children: ReactNode }) {
  const audio = useRef<HTMLAudioElement>(null);
  const [queue, setQueue] = useState<QueueItem[]>([]);
  const [index, setIndex] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [time, setTime] = useState(0);
  const [duration, setDuration] = useState(0);
  const [failed, setFailed] = useState(false);
  const current = queue[index];

  const play = useCallback((q: QueueItem[], i: number) => {
    setQueue(q);
    setIndex(i);
    setFailed(false);
  }, []);
  const toggle = useCallback(() => {
    const a = audio.current;
    if (!a) return;
    if (a.paused) void a.play().catch(() => setFailed(true));
    else a.pause();
  }, []);
  const step = useCallback(
    (d: 1 | -1) => {
      const a = audio.current;
      // back restarts the track unless it just began
      if (d < 0 && a && a.currentTime > 3) {
        a.currentTime = 0;
        return;
      }
      setIndex((i) => Math.min(Math.max(i + d, 0), queue.length - 1));
      setFailed(false);
    },
    [queue.length],
  );
  const close = () => {
    audio.current?.pause();
    setQueue([]);
    setIndex(0);
  };

  // load and start whenever the track changes
  useEffect(() => {
    const a = audio.current;
    if (!a || !current) return;
    a.src = streamUrl(current.fileId);
    void a.play().catch(() => setFailed(true));
  }, [current]);

  // keyboard media keys and the system's now-playing panel
  useEffect(() => {
    if (!("mediaSession" in navigator) || !current) return;
    navigator.mediaSession.metadata = new MediaMetadata({
      title: current.title,
      artist: current.artist,
      album: current.album,
      artwork: current.mbid ? [{ src: cover(current.mbid) ?? "", sizes: "500x500", type: "image/jpeg" }] : [],
    });
    navigator.mediaSession.setActionHandler("play", toggle);
    navigator.mediaSession.setActionHandler("pause", toggle);
    navigator.mediaSession.setActionHandler("previoustrack", () => step(-1));
    navigator.mediaSession.setActionHandler("nexttrack", () => step(1));
  }, [current, toggle, step]);

  const value = useMemo(() => ({ current, playing, play, toggle }), [current, playing, play, toggle]);

  return (
    <PlayerContext.Provider value={value}>
      {children}
      <audio
        ref={audio}
        preload="auto"
        onPlay={() => setPlaying(true)}
        onPause={() => setPlaying(false)}
        onTimeUpdate={(e) => setTime(e.currentTarget.currentTime)}
        onDurationChange={(e) => setDuration(e.currentTarget.duration)}
        onEnded={() => (index < queue.length - 1 ? step(1) : setPlaying(false))}
        onError={() => setFailed(true)}
      />
      {current && (
        <>
          <div aria-hidden className="h-24" />
          <section
            aria-label="Now playing"
            className="fixed inset-x-0 bottom-0 z-30 border-t border-line bg-sunk/95 backdrop-blur-xl md:left-64"
          >
            <input
              type="range"
              min={0}
              max={duration || 0}
              step={0.1}
              value={Math.min(time, duration || 0)}
              onChange={(e) => {
                if (audio.current) audio.current.currentTime = Number(e.target.value);
              }}
              aria-label="Position"
              className="seek absolute inset-x-0 -top-[7px] w-full"
              style={{ "--seek": `${duration ? (time / duration) * 100 : 0}%` } as CSSProperties}
            />
            <div className="flex h-20 items-center gap-3 px-4 sm:gap-4 sm:px-6">
              <div className="relative size-12 shrink-0 overflow-hidden rounded bg-raised">
                <Disc3 aria-hidden className="absolute inset-0 m-auto size-5 text-faint" strokeWidth={1.5} />
                {current.mbid && (
                  <img
                    src={cover(current.mbid)}
                    alt=""
                    className="relative size-full object-cover"
                    onError={(e) => (e.currentTarget.style.display = "none")}
                  />
                )}
              </div>
              <div className="min-w-0 flex-1">
                <p className="truncate font-semibold">{current.title}</p>
                <p className="truncate text-sm text-muted">
                  {failed ? <span className="text-danger">This file could not be played</span> : current.artist}
                </p>
              </div>
              <span className="data hidden text-xs text-faint tabular-nums sm:block">
                {clock(time)} / {clock(duration)}
              </span>
              <div className="flex items-center gap-1">
                <button type="button" onClick={() => step(-1)} className="player-btn" aria-label="Previous track">
                  <SkipBack aria-hidden className="size-5" />
                </button>
                <button
                  type="button"
                  onClick={toggle}
                  className="grid size-11 place-items-center rounded-full bg-fg text-canvas transition hover:scale-105"
                  aria-label={playing ? "Pause" : "Play"}
                >
                  {playing ? <Pause aria-hidden className="size-5" fill="currentColor" /> : <Play aria-hidden className="size-5 translate-x-px" fill="currentColor" />}
                </button>
                <button
                  type="button"
                  onClick={() => step(1)}
                  disabled={index >= queue.length - 1}
                  className="player-btn"
                  aria-label="Next track"
                >
                  <SkipForward aria-hidden className="size-5" />
                </button>
              </div>
              <button type="button" onClick={close} className="player-btn" aria-label="Close player">
                <X aria-hidden className="size-5" />
              </button>
            </div>
          </section>
        </>
      )}
    </PlayerContext.Provider>
  );
}
