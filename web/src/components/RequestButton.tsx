import { Check, Plus } from "lucide-react";
import type { Schemas } from "../api/client";
import { useRequestFilm, type RequestKind } from "../api/queries";
import { STATE_HINT, isTitleState } from "../lib/states";
import { StateBadge } from "./StateBadge";

/**
 * The one action on a film or show. Unknown films get the accent button; known ones
 * show their state and what happens next.
 */
export function RequestButton({
  tmdbId,
  local,
  kind = "movie",
  mbid,
  olid,
  label,
}: {
  tmdbId: number;
  local?: Schemas["LocalState"] | null;
  kind?: RequestKind;
  /** MusicBrainz id, for albums. */
  mbid?: string;
  /** Open Library id, for ebooks and audiobooks. */
  olid?: string;
  /** What is requested, when a page offers more than one ("ebook"). */
  label?: string;
}) {
  const request = useRequestFilm();
  const state = request.data?.title_state ?? local?.state;

  if (state) {
    return (
      <div className="flex items-center gap-3" aria-live="polite">
        {label && <span className="text-sm font-semibold capitalize">{label}</span>}
        {state === "available" ? (
          <span className="inline-flex items-center gap-2 font-bold text-state-available">
            <Check aria-hidden className="size-5" /> In your library
          </span>
        ) : (
          <>
            <StateBadge state={state} />
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
        onClick={() => request.mutate({ tmdbId, kind, mbid, olid })}
      >
        <Plus aria-hidden className="size-4" strokeWidth={2.5} />
        {request.isPending ? "Requesting…" : label ? `Request ${label}` : "Request"}
      </button>
      {request.error && (
        <p role="alert" className="text-sm text-danger">
          {request.error.message}
        </p>
      )}
    </div>
  );
}
