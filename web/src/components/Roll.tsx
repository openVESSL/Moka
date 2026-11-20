import { Link } from "@tanstack/react-router";
import { Check, ChevronLeft, ChevronRight, Film } from "lucide-react";
import { useRef, type ReactNode } from "react";
import { cover, img } from "../api/client";
import { Tape } from "./Tape";

export type RollItem = {
  key: string | number;
  tmdbId?: number | null;
  title: string;
  year?: number | null;
  posterPath?: string | null;
  state?: string | null;
  kind?: "movie" | "show" | "album";
  /** MusicBrainz id, for albums (cover and link). */
  mbid?: string | null;
};

const GRID =
  "grid grid-cols-[repeat(auto-fill,minmax(8.5rem,1fr))] gap-x-4 gap-y-7 sm:grid-cols-[repeat(auto-fill,minmax(10rem,1fr))]";

/** A poster grid. Titles on the way carry a state badge; ones in the library a check. */
export function Roll({ items }: { items: RollItem[] }) {
  return (
    <ol className={GRID}>
      {items.map((it) => (
        <li key={it.key}>
          <RollCell item={it} />
        </li>
      ))}
    </ol>
  );
}

/** One horizontal row of posters with a title, scrolled sideways (arrows on wide screens). */
export function PosterRow({ title, action, items }: { title: string; action?: ReactNode; items?: RollItem[] }) {
  const scroller = useRef<HTMLOListElement>(null);
  const scroll = (dir: 1 | -1) =>
    scroller.current?.scrollBy({ left: dir * scroller.current.clientWidth * 0.85, behavior: "smooth" });
  return (
    <section className="group/row">
      <div className="mb-3 flex items-center gap-3">
        <h2 className="text-lg font-semibold">{title}</h2>
        {action}
        <div className="ml-auto hidden gap-1 md:flex">
          {([-1, 1] as const).map((d) => (
            <button
              key={d}
              type="button"
              onClick={() => scroll(d)}
              className="grid size-8 place-items-center rounded-full text-muted opacity-0 transition hover:bg-white/10 hover:text-fg group-hover/row:opacity-100 focus-visible:opacity-100"
              aria-label={d < 0 ? "Scroll back" : "Scroll forward"}
            >
              {d < 0 ? <ChevronLeft aria-hidden className="size-5" /> : <ChevronRight aria-hidden className="size-5" />}
            </button>
          ))}
        </div>
      </div>
      <ol
        ref={scroller}
        className="no-scrollbar -mx-4 flex snap-x scroll-px-4 gap-4 overflow-x-auto px-4 pt-1 pb-3 sm:-mx-8 sm:scroll-px-8 sm:px-8"
      >
        {items
          ? items.map((it) => (
              <li key={it.key} className="w-36 shrink-0 snap-start sm:w-40">
                <RollCell item={it} />
              </li>
            ))
          : Array.from({ length: 10 }, (_, i) => (
              <li key={i} aria-hidden className="w-36 shrink-0 sm:w-40">
                <div className="aspect-[2/3] animate-pulse rounded-xl bg-raised" />
              </li>
            ))}
      </ol>
    </section>
  );
}

function RollCell({ item }: { item: RollItem }) {
  const square = item.kind === "album";
  const inner = (
    <>
      <div
        className={`relative overflow-hidden rounded-xl bg-raised shadow-[0_10px_30px_-12px_rgb(0_0_0/0.8)] ring-1 ring-white/[0.07] transition duration-300 ease-[var(--ease-out-expo)] group-hover:scale-[1.035] group-hover:ring-white/25 ${square ? "aspect-square" : "aspect-[2/3]"}`}
      >
        {item.posterPath || item.mbid ? (
          <img
            src={item.mbid ? cover(item.mbid) : img("w342", item.posterPath)}
            alt=""
            loading="lazy"
            decoding="async"
            className="size-full object-cover"
          />
        ) : (
          <div className="grid size-full place-items-center p-3 text-center">
            <Film aria-hidden className="size-6 text-faint" strokeWidth={1.5} />
          </div>
        )}
        <div
          aria-hidden
          className="absolute inset-0 bg-gradient-to-t from-black/60 via-transparent to-transparent opacity-0 transition-opacity duration-300 group-hover:opacity-100"
        />
        {item.state && item.state !== "available" && <Tape state={item.state} className="absolute top-2 left-2" />}
        {item.state === "available" && (
          <span
            className="absolute top-2 right-2 grid size-5 place-items-center rounded-full bg-state-available text-black shadow"
            title="In your library"
          >
            <Check aria-hidden className="size-3" strokeWidth={3} />
            <span className="sr-only">In your library</span>
          </span>
        )}
      </div>
      <div className="mt-2.5 min-w-0 px-0.5">
        <span className="line-clamp-1 text-sm font-medium text-fg">{item.title}</span>
        {item.year && <span className="data text-xs text-faint">{item.year}</span>}
      </div>
    </>
  );

  const label = item.year ? `${item.title} (${item.year})` : item.title;
  if (item.kind === "album" && item.mbid) {
    return (
      <Link to="/album/$mbid" params={{ mbid: item.mbid }} className="group block rounded-xl" aria-label={label}>
        {inner}
      </Link>
    );
  }
  if (!item.tmdbId) return <div className="group">{inner}</div>;
  return (
    <Link
      to={item.kind === "show" ? "/show/$tmdbId" : "/movie/$tmdbId"}
      params={{ tmdbId: String(item.tmdbId) }}
      className="group block rounded-xl"
      aria-label={label}
    >
      {inner}
    </Link>
  );
}

export function RollSkeleton({ count = 18 }: { count?: number }) {
  return (
    <div aria-hidden className={GRID}>
      {Array.from({ length: count }, (_, i) => (
        <div key={i}>
          <div className="aspect-[2/3] animate-pulse rounded-xl bg-raised" />
          <div className="mt-2.5 h-3 w-3/4 rounded bg-raised" />
        </div>
      ))}
    </div>
  );
}
