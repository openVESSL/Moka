import { useQuery } from "@tanstack/react-query";
import { ArrowUpRight } from "lucide-react";
import { img } from "../api/client";
import { meQuery, movieQuery } from "../api/queries";
import { RequestButton } from "../components/RequestButton";
import { Roll } from "../components/Roll";
import { SlateFields } from "../components/SlateFields";
import { TitlePanel } from "../components/TitlePanel";
import { runtime } from "../lib/states";
import { CatalogError } from "./discover";

export function MoviePage({ tmdbId }: { tmdbId: number }) {
  const { data: m, error, isPending } = useQuery(movieQuery(tmdbId));
  const { data: me } = useQuery(meQuery);
  if (error) return <CatalogError error={error} />;
  if (isPending) return <div className="h-[70vh] animate-pulse bg-raised/50" />;

  return (
    <main>
      <section className="relative isolate overflow-hidden">
        {m.backdrop_path && (
          <img
            src={img("w1280", m.backdrop_path)}
            alt=""
            className="absolute inset-0 -z-20 size-full object-cover object-[center_20%]"
          />
        )}
        <div
          aria-hidden
          className="absolute inset-0 -z-10 bg-[linear-gradient(to_top,var(--color-canvas)_6%,rgb(12_12_14/0.88)_45%,rgb(12_12_14/0.35)),linear-gradient(to_right,rgb(12_12_14/0.85),transparent_70%)]"
        />
        <div className="mx-auto grid max-w-[110rem] gap-10 px-4 pt-16 pb-8 sm:px-8 md:grid-cols-[minmax(0,15rem)_1fr] md:items-end lg:pt-28">
          {m.poster_path && (
            <img
              src={img("w500", m.poster_path)}
              alt={`${m.title} poster`}
              className="hidden w-full rounded-md shadow-[0_24px_40px_-18px_rgb(0_0_0/0.95)] md:block"
            />
          )}
          <div className="min-w-0">
            <h1 className="max-w-[18ch] text-[clamp(2rem,4.5vw,3.5rem)] leading-[1.05] font-bold tracking-[-0.03em]">
              {m.title}
            </h1>
            {m.original_title && m.original_title !== m.title && (
              <p className="mt-2 text-muted italic">{m.original_title}</p>
            )}
            {m.tagline && <p className="mt-4 max-w-[52ch] text-lg text-muted">{m.tagline}</p>}
            <SlateFields
              className="mt-7 max-w-3xl"
              fields={[
                { label: "Director", value: m.directors.slice(0, 2).join(", ") },
                { label: "Year", value: m.year, mono: true },
                { label: "Runtime", value: runtime(m.runtime_minutes), mono: true },
                { label: "Rating", value: m.vote_average ? m.vote_average.toFixed(1) : undefined, mono: true },
              ]}
            />
            <div className="mt-7">
              <RequestButton tmdbId={m.tmdb_id} local={m.local} />
            </div>
          </div>
        </div>
      </section>

      <div className="mx-auto grid max-w-[110rem] gap-10 px-4 sm:px-8 xl:grid-cols-[minmax(0,42rem)_minmax(0,1fr)]">
        <section aria-labelledby="story">
          <h2 id="story" className="field-label mb-3">
            Story
          </h2>
          <p className="max-w-[65ch] text-[17px] leading-relaxed text-fg">{m.overview || "No synopsis yet."}</p>
          {m.genres.length > 0 && (
            <p className="mt-5 text-sm font-semibold text-muted">
              {m.genres.join(" · ")}
            </p>
          )}
          {m.trailer_youtube_key && (
            <a
              href={`https://www.youtube.com/watch?v=${m.trailer_youtube_key}`}
              target="_blank"
              rel="noreferrer noopener"
              className="btn mt-7"
            >
              Watch trailer <ArrowUpRight aria-hidden className="size-4" />
            </a>
          )}
        </section>

        {m.cast.length > 0 && (
          <section aria-labelledby="cast">
            <h2 id="cast" className="field-label mb-3">
              Cast
            </h2>
            <ul className="grid grid-cols-1 gap-x-6 sm:grid-cols-2">
              {m.cast.map((p) => (
                <li key={p.id} className="flex min-w-0 items-baseline justify-between gap-4 border-b border-line py-2.5">
                  <span className="truncate font-semibold">{p.name}</span>
                  <span className="shrink-0 truncate text-right text-sm text-faint max-w-[50%]">{p.role}</span>
                </li>
              ))}
            </ul>
          </section>
        )}
      </div>

      {m.local && <TitlePanel titleId={m.local.title_id} permissions={me?.permissions ?? 0} />}

      {m.recommendations.length > 0 && (
        <section aria-labelledby="more" className="mx-auto mt-10 max-w-[110rem] px-4 sm:px-8">
          <h2 id="more" className="mb-8 text-lg font-semibold">
            If you liked this
          </h2>
          <Roll
           
            items={m.recommendations.map((r) => ({
              key: r.tmdb_id,
              tmdbId: r.tmdb_id,
              title: r.title,
              year: r.year,
              posterPath: r.poster_path,
              state: r.local?.state,
            }))}
          />
        </section>
      )}
    </main>
  );
}
