import { useRouter } from "@tanstack/react-router";
import { ArrowLeft } from "lucide-react";
import { useState } from "react";

/**
 * Direct play: the original file over HTTP. Browsers play MP4/WebM and many
 * MKV files with H.264/AAC; anything else needs transcoding, which comes next.
 */
export function WatchPage({ fileId }: { fileId: number }) {
  const router = useRouter();
  const [failed, setFailed] = useState(false);

  return (
    <main className="fixed inset-0 z-50 grid place-items-center bg-black">
      <button
        type="button"
        onClick={() => router.history.back()}
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
          src={`/api/v1/files/${fileId}/stream`}
          controls
          autoPlay
          playsInline
          onError={() => setFailed(true)}
          className="size-full bg-black"
        />
      )}
    </main>
  );
}
