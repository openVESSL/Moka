import { Link } from "@tanstack/react-router";
import { Film } from "lucide-react";
import { img } from "../api/client";
import { GreaseCircle } from "./GreaseCircle";
import { Tape } from "./Tape";

export type RollItem = {
  key: string | number;
  tmdbId?: number | null;
  title: string;
  year?: number | null;
  posterPath?: string | null;
  state?: string | null;
  kind?: "movie" | "show";
};

/**
 * The roll: a dense grid of films, each numbered like a take. Films already
 * in the library get their number circled; films on the way carry a tape.
 */
export function Roll({ items, numbered = true }: { items: RollItem[]; numbered?: boolean }) {
  return (
    <ol className="grid grid-cols-[repeat(auto-fill,minmax(8.25rem,1fr))] gap-x-4 gap-y-7 sm:grid-cols-[repeat(auto-fill,minmax(9.5rem,1fr))]">
      {items.map((it, i) => (
        <li key={it.key}>
          <RollCell item={it} take={numbered ? i + 1 : undefined} />
        </li>
      ))}
    </ol>
  );
}

function RollCell({ item, take }: { item: RollItem; take?: number }) {
  const inner = (
    <>
      <div className="relative aspect-[2/3] overflow-hidden rounded-[3px] bg-slate-raised shadow-[0_8px_18px_-10px_rgb(0_0_0/0.9)] transition-transform duration-300 ease-[var(--ease-out-expo)] group-hover:-translate-y-1">
        {item.posterPath ? (
          <img
            src={img("w342", item.posterPath)}
            alt=""
            loading="lazy"
            decoding="async"
            className="size-full object-cover"
          />
        ) : (
          <div className="grid size-full place-items-center p-3 text-center">
            <Film aria-hidden className="size-6 text-chalk-faint" strokeWidth={1.5} />
          </div>
        )}
        {item.state && item.state !== "available" && (
          <Tape state={item.state} className="absolute top-2 -left-1 -rotate-3" />
        )}
      </div>
      <div className="mt-2 flex items-baseline gap-2">
        {take !== undefined && (
          <span className="relative shrink-0 px-1 data text-[11px] text-chalk-faint">
            {String(take).padStart(2, "0")}
            {item.state === "available" && <GreaseCircle className="-inset-x-1.5 -inset-y-1" />}
          </span>
        )}
        <span className="min-w-0">
          <span className="line-clamp-2 font-slate text-[15px] leading-tight font-semibold text-chalk group-hover:text-white">
            {item.title}
          </span>
          {item.year && <span className="data text-[11px] text-chalk-faint">{item.year}</span>}
        </span>
      </div>
    </>
  );

  if (!item.tmdbId) return <div className="group">{inner}</div>;
  return (
    <Link
      to={item.kind === "show" ? "/show/$tmdbId" : "/movie/$tmdbId"}
      params={{ tmdbId: String(item.tmdbId) }}
      className="group block rounded-[3px] focus-visible:outline-offset-4"
      aria-label={item.year ? `${item.title} (${item.year})` : item.title}
    >
      {inner}
    </Link>
  );
}

export function RollSkeleton({ count = 18 }: { count?: number }) {
  return (
    <div
      aria-hidden
      className="grid grid-cols-[repeat(auto-fill,minmax(8.25rem,1fr))] gap-x-4 gap-y-7 sm:grid-cols-[repeat(auto-fill,minmax(9.5rem,1fr))]"
    >
      {Array.from({ length: count }, (_, i) => (
        <div key={i}>
          <div className="aspect-[2/3] animate-pulse rounded-[3px] bg-slate-raised" />
          <div className="mt-2 h-3 w-3/4 rounded bg-slate-raised" />
        </div>
      ))}
    </div>
  );
}
