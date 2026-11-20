import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ArrowUpRight, ChevronDown } from "lucide-react";
import { useState } from "react";
import { api, img, unwrap, type Schemas } from "../api/client";
import { episodesQuery, meQuery, showQuery } from "../api/queries";
import { RequestButton } from "../components/RequestButton";
import { Facts } from "../components/Facts";
import { StateBadge } from "../components/StateBadge";
import { TitlePanel } from "../components/TitlePanel";
import { runtime } from "../lib/states";
import { CatalogError } from "./discover";

export function ShowPage({ tmdbId }: { tmdbId: number }) {
  const { data: s, error, isPending } = useQuery(showQuery(tmdbId));
  if (error) return <CatalogError error={error} />;
  if (isPending) return <div className="h-[70vh] animate-pulse bg-raised/50" />;
  const seasons = s.seasons.filter((x) => x.season_number > 0).length;

  return (
    <main>
      <section className="relative isolate overflow-hidden">
        {s.backdrop_path && (
          <img
            src={img("w1280", s.backdrop_path)}
            alt=""
            className="absolute inset-0 -z-20 size-full object-cover object-[center_20%]"
          />
        )}
        <div
          aria-hidden
          className="absolute inset-0 -z-10 bg-[linear-gradient(to_top,var(--color-canvas)_6%,rgb(12_12_14/0.88)_45%,rgb(12_12_14/0.35)),linear-gradient(to_right,rgb(12_12_14/0.85),transparent_70%)]"
        />
        <div className="mx-auto grid max-w-[110rem] gap-10 px-4 pt-16 pb-8 sm:px-8 md:grid-cols-[minmax(0,15rem)_1fr] md:items-end lg:pt-28">
          {s.poster_path && (
            <img
              src={img("w500", s.poster_path)}
              alt={`${s.title} poster`}
              className="hidden w-full rounded-md shadow-[0_24px_40px_-18px_rgb(0_0_0/0.95)] md:block"
            />
          )}
          <div className="min-w-0">
            <h1 className="max-w-[18ch] text-[clamp(2rem,4.5vw,3.5rem)] leading-[1.05] font-bold tracking-[-0.03em]">
              {s.title}
            </h1>
            {s.original_title && s.original_title !== s.title && (
              <p className="mt-2 text-muted italic">{s.original_title}</p>
            )}
            {s.tagline && <p className="mt-4 max-w-[52ch] text-lg text-muted">{s.tagline}</p>}
            <Facts
              className="mt-7 max-w-3xl"
              fields={[
                { label: "Year", value: s.year, mono: true },
                { label: "Seasons", value: seasons || undefined, mono: true },
                { label: "Status", value: s.status ?? undefined },
                { label: "Episode", value: runtime(s.runtime_minutes), mono: true },
                { label: "Rating", value: s.vote_average ? s.vote_average.toFixed(1) : undefined, mono: true },
              ]}
            />
            <div className="mt-7">
              <RequestButton tmdbId={s.tmdb_id} local={s.local} kind="show" />
            </div>
          </div>
        </div>
      </section>

      <div className="mx-auto grid max-w-[110rem] gap-10 px-4 sm:px-8 xl:grid-cols-[minmax(0,42rem)_minmax(0,1fr)]">
        <section aria-labelledby="story">
          <h2 id="story" className="field-label mb-3">
            Story
          </h2>
          <p className="max-w-[65ch] text-[17px] leading-relaxed text-fg">{s.overview || "No synopsis yet."}</p>
          {s.genres.length > 0 && (
            <p className="mt-5 text-sm font-semibold text-muted">
              {s.genres.join(" · ")}
            </p>
          )}
          {s.trailer_youtube_key && (
            <a
              href={`https://www.youtube.com/watch?v=${s.trailer_youtube_key}`}
              target="_blank"
              rel="noreferrer noopener"
              className="btn mt-7"
            >
              Watch trailer <ArrowUpRight aria-hidden className="size-4" />
            </a>
          )}
        </section>

        {s.cast.length > 0 && (
          <section aria-labelledby="cast">
            <h2 id="cast" className="field-label mb-3">
              Cast
            </h2>
            <ul className="grid grid-cols-1 gap-x-6 sm:grid-cols-2">
              {s.cast.map((p) => (
                <li key={p.id} className="flex min-w-0 items-baseline justify-between gap-4 border-b border-line py-2.5">
                  <span className="truncate font-semibold">{p.name}</span>
                  <span className="shrink-0 truncate text-right text-sm text-faint max-w-[50%]">{p.role}</span>
                </li>
              ))}
            </ul>
          </section>
        )}
      </div>

      <section aria-labelledby="seasons" className="mx-auto mt-10 max-w-[110rem] px-4 sm:px-8">
        <h2 id="seasons" className="mb-8 text-lg font-semibold">
          Seasons
        </h2>
        {s.local ? <Episodes titleId={s.local.title_id} /> : <CatalogSeasons seasons={s.seasons} />}
      </section>
    </main>
  );
}

function CatalogSeasons({ seasons }: { seasons: Schemas["CatalogSeason"][] }) {
  return (
    <ul className="border-t border-line">
      {seasons.map((x) => (
        <li key={x.season_number} className="flex items-baseline justify-between gap-4 border-b border-line py-3">
          <span className="text-lg font-semibold">{x.name}</span>
          <span className="data text-sm text-faint">
            {x.episode_count} episodes{x.air_date ? ` · ${x.air_date.slice(0, 4)}` : ""}
          </span>
        </li>
      ))}
    </ul>
  );
}

/** Episodes in the library, by season, each with where it stands. */
function Episodes({ titleId }: { titleId: number }) {
  const { data, isPending } = useQuery({ ...episodesQuery(titleId), refetchInterval: 15_000 });
  const { data: me } = useQuery(meQuery);
  const [open, setOpen] = useState<number | null>(null);
  const qc = useQueryClient();
  const canManage = ((me?.permissions ?? 0) & (1 | 16)) !== 0;
  const follow = useMutation({
    mutationFn: ({ id, monitored }: { id: number; monitored: boolean }) =>
      unwrap(api.PUT("/api/v1/titles/{id}/monitored", { params: { path: { id } }, body: { monitored } })),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["titles", titleId, "episodes"] }),
  });
  if (isPending) return <div className="h-40 animate-pulse rounded bg-raised/50" />;
  const bySeason = new Map<number, Schemas["Title"][]>();
  for (const e of data ?? []) {
    const n = e.season_number ?? 0;
    bySeason.set(n, [...(bySeason.get(n) ?? []), e]);
  }
  return (
    <div className="grid gap-10">
      {[...bySeason].map(([n, eps]) => (
        <section key={n} aria-labelledby={`season-${n}`}>
          <div className="mb-3 flex flex-wrap items-baseline gap-x-4 gap-y-2">
            <h3 id={`season-${n}`} className="field-label">
              {n === 0 ? "Specials" : `Season ${n}`}
              <span className="ml-3 data text-faint">
                {eps.filter((e) => e.state === "available").length}/{eps.length}
              </span>
            </h3>
            {canManage && eps[0]?.parent_id && eps.some((e) => !e.monitored && e.state !== "available") && (
              <button
                type="button"
                className="text-xs font-bold text-accent hover:text-accent-hot disabled:opacity-50"
                disabled={follow.isPending}
                onClick={() => follow.mutate({ id: eps[0].parent_id!, monitored: true })}
              >
                Get missing episodes
              </button>
            )}
          </div>
          <ol className="border-t border-line">
            {eps.map((e) => (
              <li key={e.id} className="border-b border-line">
                <div className="flex items-center gap-3">
                  <button
                    type="button"
                    aria-expanded={open === e.id}
                    onClick={() => setOpen(open === e.id ? null : e.id)}
                    className="flex min-w-0 flex-1 items-center gap-4 py-2.5 text-left hover:text-white"
                  >
                    <span className="w-10 shrink-0 data text-sm text-faint">
                      {String(e.episode_number ?? 0).padStart(2, "0")}
                    </span>
                    <span className="min-w-0 flex-1 truncate text-[17px] font-semibold">{e.title}</span>
                    {e.release_date && (
                      <span className="data hidden text-xs text-faint sm:inline">{e.release_date}</span>
                    )}
                    {!e.monitored ? (
                      <span className="text-xs text-faint">Not monitored</span>
                    ) : (
                      e.state !== "available" && <StateBadge state={e.state} />
                    )}
                    <ChevronDown
                      aria-hidden
                      className={`size-4 shrink-0 text-faint transition-transform ${open === e.id ? "rotate-180" : ""}`}
                    />
                  </button>
                  {canManage && !e.monitored && e.state !== "available" && (
                    <button
                      type="button"
                      className="shrink-0 text-xs font-bold text-accent hover:text-accent-hot disabled:opacity-50"
                      disabled={follow.isPending}
                      onClick={() => follow.mutate({ id: e.id, monitored: true })}
                    >
                      Get this one
                    </button>
                  )}
                </div>
                {open === e.id && (
                  <div className="pb-6 [&>section]:mt-0 [&>section]:px-0">
                    <TitlePanel titleId={e.id} permissions={me?.permissions ?? 0} />
                  </div>
                )}
              </li>
            ))}
          </ol>
        </section>
      ))}
    </div>
  );
}
