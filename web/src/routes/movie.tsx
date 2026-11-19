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
  if (isPending) return <div className="h-[70vh] animate-pulse bg-slate-raised/50" />;

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
          className="absolute inset-0 -z-10 bg-[linear-gradient(to_top,var(--color-slate)_6%,rgb(20_21_22/0.88)_45%,rgb(20_21_22/0.35)),linear-gradient(to_right,rgb(20_21_22/0.85),transparent_70%)]"
        />
        <div className="mx-auto grid max-w-[110rem] gap-10 px-4 pt-28 pb-12 sm:px-8 md:grid-cols-[minmax(0,15rem)_1fr] md:items-end lg:pt-48">
          {m.poster_path && (
            <img
              src={img("w500", m.poster_path)}
              alt={`${m.title} poster`}
              className="hidden w-full rounded-[3px] shadow-[0_24px_40px_-18px_rgb(0_0_0/0.95)] md:block"
            />
          )}
          <div className="min-w-0">
            <h1 className="max-w-[18ch] font-slate text-[clamp(2.75rem,7vw,5.5rem)] leading-[0.88] font-extrabold tracking-[-0.02em] uppercase">
              {m.title}
            </h1>
            {m.original_title && m.original_title !== m.title && (
              <p className="mt-2 text-chalk-dim italic">{m.original_title}</p>
            )}
            {m.tagline && <p className="mt-4 max-w-[52ch] text-lg text-chalk-dim">{m.tagline}</p>}
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

      <div className="mx-auto grid max-w-[110rem] gap-14 px-4 sm:px-8 lg:grid-cols-[minmax(0,42rem)_1fr]">
        <section aria-labelledby="story">
          <h2 id="story" className="slate-label mb-3">
            Story
          </h2>
          <p className="max-w-[65ch] text-[17px] leading-relaxed text-chalk">{m.overview || "No synopsis yet."}</p>
          {m.genres.length > 0 && (
            <p className="mt-5 font-slate text-sm font-semibold tracking-[0.08em] text-chalk-dim uppercase">
              {m.genres.join(" · ")}
            </p>
          )}
          {m.trailer_youtube_key && (
            <a
              href={`https://www.youtube.com/watch?v=${m.trailer_youtube_key}`}
              target="_blank"
              rel="noreferrer noopener"
              className="btn-chalk mt-7"
            >
              Watch trailer <ArrowUpRight aria-hidden className="size-4" />
            </a>
          )}
        </section>

        {m.cast.length > 0 && (
          <section aria-labelledby="cast">
            <h2 id="cast" className="slate-label mb-3">
              Cast
            </h2>
            <ul className="grid grid-cols-1 border-t border-slate-line sm:grid-cols-2">
              {m.cast.map((p) => (
                <li key={p.id} className="flex items-baseline justify-between gap-4 border-b border-slate-line py-2.5">
                  <span className="font-slate text-[17px] font-semibold">{p.name}</span>
                  <span className="truncate text-right text-sm text-chalk-faint">{p.role}</span>
                </li>
              ))}
            </ul>
          </section>
        )}
      </div>

      {m.local && <TitlePanel titleId={m.local.title_id} permissions={me?.permissions ?? 0} />}

      {m.recommendations.length > 0 && (
        <section aria-labelledby="more" className="mx-auto mt-20 max-w-[110rem] px-4 sm:px-8">
          <h2 id="more" className="mb-8 font-slate text-3xl font-extrabold uppercase">
            If you liked this
          </h2>
          <Roll
            numbered={false}
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
