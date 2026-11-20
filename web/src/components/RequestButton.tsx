import { Check, Plus } from "lucide-react";
import type { Schemas } from "../api/client";
import { useRequestFilm, type RequestKind } from "../api/queries";
import { STATE_HINT, isTitleState } from "../lib/states";
import { Tape } from "./Tape";

/**
 * The one action on a film or show. Unknown films get the accent button; known ones
 * show their tape and what happens next.
 */
export function RequestButton({
  tmdbId,
  local,
  kind = "movie",
  mbid,
}: {
  tmdbId: number;
  local?: Schemas["LocalState"] | null;
  kind?: RequestKind;
  /** MusicBrainz id, for albums. */
  mbid?: string;
}) {
  const request = useRequestFilm();
  const state = request.data?.title_state ?? local?.state;

  if (state) {
    return (
      <div className="flex items-center gap-3" aria-live="polite">
        {state === "available" ? (
          <span className="inline-flex items-center gap-2 font-bold text-state-available">
            <Check aria-hidden className="size-5" /> In your library
          </span>
        ) : (
          <>
            <Tape state={state} />
            <span className="text-sm text-muted">{isTitleState(state) ? STATE_HINT[state] : null}</span>
          </>
        )}
      </div>
    );
  }

  return (
    <div className="flex flex-wrap items-center gap-3">
      <button
        type="button"
        className="btn-primary"
        disabled={request.isPending}
        onClick={() => request.mutate({ tmdbId, kind, mbid })}
      >
        <Plus aria-hidden className="size-4" strokeWidth={2.5} />
        {request.isPending ? "Requesting…" : "Request"}
      </button>
      {request.error && (
        <p role="alert" className="text-sm text-danger">
          {request.error.message}
        </p>
      )}
    </div>
  );
}
