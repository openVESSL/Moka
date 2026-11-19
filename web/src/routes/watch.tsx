import { useRouter } from "@tanstack/react-router";
import { ArrowLeft } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { api } from "../api/client";

const SAVE_EVERY_MS = 10_000;

/**
 * Direct play: the original file over HTTP. Browsers play MP4/WebM and many
 * MKV files with H.264/AAC; anything else needs transcoding, which comes next.
 * Picks up where the viewer left off and saves the position as it plays.
 */
export function WatchPage({ fileId }: { fileId: number }) {
  const router = useRouter();
  const video = useRef<HTMLVideoElement>(null);
  const [failed, setFailed] = useState(false);
  const resumeAt = useRef<number | null>(null);
  const lastSave = useRef(0);

  useEffect(() => {
    api.GET("/api/v1/files/{id}/progress", { params: { path: { id: fileId } } }).then(({ data }) => {
      if (data && !data.played && data.position_seconds > 30) resumeAt.current = data.position_seconds;
    });
  }, [fileId]);

  function save(force = false) {
    const v = video.current;
    if (!v || !Number.isFinite(v.duration) || v.duration === 0) return;
    const now = Date.now();
    if (!force && now - lastSave.current < SAVE_EVERY_MS) return;
    lastSave.current = now;
    // the last few minutes are credits: count the film as watched
    const played = v.currentTime / v.duration > 0.92;
    api.PUT("/api/v1/files/{id}/progress", {
      params: { path: { id: fileId } },
      body: { position_seconds: played ? 0 : v.currentTime, played },
    });
  }

  return (
    <main className="fixed inset-0 z-50 grid place-items-center bg-black">
      <button
        type="button"
        onClick={() => {
          save(true);
          router.history.back();
        }}
        className="absolute top-4 left-4 z-10 inline-flex items-center gap-2 rounded-[3px] bg-black/50 px-3 py-2 font-slate text-sm font-semibold tracking-[0.08em] text-chalk uppercase hover:bg-black/80"
      >
        <ArrowLeft aria-hidden className="size-4" /> Back
      </button>
      {failed ? (
        <div className="max-w-md px-6 text-center">
          <h1 className="font-slate text-3xl font-extrabold uppercase">This browser cannot play the file as is</h1>
          <p className="mt-3 text-chalk-dim">
            Its video or audio format needs converting on the fly. Converting is not available in this version yet;
            the file plays in apps that read it directly.
          </p>
          <a href={`/api/v1/files/${fileId}/stream`} className="btn-chalk mt-6" download>
            Download file
          </a>
        </div>
      ) : (
        <video
          ref={video}
          src={`/api/v1/files/${fileId}/stream`}
          controls
          autoPlay
          playsInline
          onLoadedMetadata={(e) => {
            if (resumeAt.current) e.currentTarget.currentTime = resumeAt.current;
          }}
          onTimeUpdate={() => save()}
          onPause={() => save(true)}
          onEnded={() => save(true)}
          onError={() => setFailed(true)}
          className="size-full bg-black"
        />
      )}
    </main>
  );
}
