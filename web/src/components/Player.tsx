import { Disc3, Pause, Play, RotateCcw, RotateCw, SkipBack, SkipForward, X } from "lucide-react";
import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { api, cover } from "../api/client";

export type QueueItem = {
  fileId: number;
  title: string;
  artist: string;
  album: string;
  /** Album MusicBrainz id, for the cover. */
  mbid?: string | null;
  /** Cover URL, when it is not an album's. */
  art?: string;
  /** An audiobook part: where to start, and the place is saved as it plays. */
  book?: { startAt?: number };
};

const SAVE_EVERY_S = 15;

/** Saves the place in an audiobook: this part, at this second. */
function saveBookPlace(fileId: number, seconds: number, finished: boolean) {
  void api.PUT("/api/v1/files/{id}/progress", {
    params: { path: { id: fileId } },
    body: { position_seconds: Math.floor(seconds), played: finished, bookmark: String(fileId) },
  });
}

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

/** Holds the music or audiobook queue and one <audio> element shared by every page. */
export function PlayerProvider({ children }: { children: ReactNode }) {
  const audio = useRef<HTMLAudioElement>(null);
  const [queue, setQueue] = useState<QueueItem[]>([]);
  const [index, setIndex] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [time, setTime] = useState(0);
  const [duration, setDuration] = useState(0);
  const [failed, setFailed] = useState(false);
  const lastSave = useRef(0);
  const current = queue[index];
  const art = current?.art ?? cover(current?.mbid);

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
  const jump = (seconds: number) => {
    const a = audio.current;
    if (a) a.currentTime = Math.max(0, Math.min(a.currentTime + seconds, a.duration || a.currentTime + seconds));
  };
  const close = () => {
    const a = audio.current;
    if (a && current?.book) saveBookPlace(current.fileId, a.currentTime, false);
    a?.pause();
    setQueue([]);
    setIndex(0);
  };

  // load and start whenever the track changes
  useEffect(() => {
    const a = audio.current;
    if (!a || !current) return;
    a.src = streamUrl(current.fileId);
    const start = current.book?.startAt;
    if (start) a.addEventListener("loadedmetadata", () => (a.currentTime = start), { once: true });
    lastSave.current = start ?? 0;
    void a.play().catch(() => setFailed(true));
  }, [current]);

  // keyboard media keys and the system's now-playing panel
  useEffect(() => {
    if (!("mediaSession" in navigator) || !current) return;
    navigator.mediaSession.metadata = new MediaMetadata({
      title: current.title,
      artist: current.artist,
      album: current.album,
      artwork: art ? [{ src: art, sizes: "500x500", type: "image/jpeg" }] : [],
    });
    navigator.mediaSession.setActionHandler("play", toggle);
    navigator.mediaSession.setActionHandler("pause", toggle);
    navigator.mediaSession.setActionHandler("previoustrack", () => step(-1));
    navigator.mediaSession.setActionHandler("nexttrack", () => step(1));
  }, [current, art, toggle, step]);

  const value = useMemo(() => ({ current, playing, play, toggle }), [current, playing, play, toggle]);

  return (
    <PlayerContext.Provider value={value}>
      {children}
      <audio
        ref={audio}
        preload="auto"
        onPlay={() => setPlaying(true)}
        onPause={(e) => {
          setPlaying(false);
          if (current?.book && !e.currentTarget.ended) saveBookPlace(current.fileId, e.currentTarget.currentTime, false);
        }}
        onTimeUpdate={(e) => {
          const t = e.currentTarget.currentTime;
          setTime(t);
          if (current?.book && Math.abs(t - lastSave.current) >= SAVE_EVERY_S) {
            lastSave.current = t;
            saveBookPlace(current.fileId, t, false);
          }
        }}
        onDurationChange={(e) => setDuration(e.currentTarget.duration)}
        onEnded={() => {
          const last = index >= queue.length - 1;
          // an audiobook carries on at the start of the next part, or is finished
          if (current?.book) saveBookPlace(last ? current.fileId : queue[index + 1].fileId, 0, last);
          if (last) setPlaying(false);
          else step(1);
        }}
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
                {art && (
                  <img
                    src={art}
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
                {current.book && (
                  <button type="button" onClick={() => jump(-15)} className="player-btn" aria-label="Back 15 seconds">
                    <RotateCcw aria-hidden className="size-5" />
                  </button>
                )}
                <button type="button" onClick={() => step(-1)} className="player-btn" aria-label={current.book ? "Previous part" : "Previous track"}>
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
                  aria-label={current.book ? "Next part" : "Next track"}
                >
                  <SkipForward aria-hidden className="size-5" />
                </button>
                {current.book && (
                  <button type="button" onClick={() => jump(30)} className="player-btn" aria-label="Forward 30 seconds">
                    <RotateCw aria-hidden className="size-5" />
                  </button>
                )}
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
