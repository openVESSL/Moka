import { useQuery } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { ApiError, img, type CatalogItem } from "../api/client";
import {
  DISCOVER_LISTS,
  SHOW_LISTS,
  discoverQuery,
  artistSearchQuery,
  searchQuery,
  type DiscoverList,
  type MediaKind,
} from "../api/queries";
import { RequestButton } from "../components/RequestButton";
import { Roll, RollSkeleton } from "../components/Roll";
import { SlateFields } from "../components/SlateFields";

const KINDS: { id: MediaKind; label: string }[] = [
  { id: "movie", label: "Films" },
  { id: "show", label: "Shows" },
];

export function DiscoverPage({ list, kind }: { list: DiscoverList; kind: MediaKind }) {
  const { data, error, isPending } = useQuery(discoverQuery(list, 1, kind));
  const navigate = useNavigate();

  if (error) return <CatalogError error={error} />;
  const items = data?.results ?? [];
  const lead = items.find((m) => m.backdrop_path) ?? items[0];
  const lists = DISCOVER_LISTS.filter((l) => kind === "movie" || SHOW_LISTS.includes(l.id));

  return (
    <main>
      {lead ? <Hero film={lead} kind={kind} /> : <div className="h-[62vh] animate-pulse bg-slate-raised/50" />}

      <section aria-labelledby="roll-heading" className="mx-auto max-w-[110rem] px-4 sm:px-8">
        <div className="mt-10 mb-8 flex flex-wrap items-end gap-x-8 gap-y-4 border-b border-slate-line">
          <h2 id="roll-heading" className="sr-only">
            {DISCOVER_LISTS.find((l) => l.id === list)?.label}
          </h2>
          <div role="tablist" aria-label="Films or shows" className="-mb-px flex gap-1">
            {KINDS.map((k) => (
              <button
                key={k.id}
                role="tab"
                aria-selected={k.id === kind}
                onClick={() => navigate({ to: "/", search: { list, kind: k.id } })}
                className="shrink-0 border-b-[3px] border-transparent px-3 pb-3 font-slate text-lg font-bold tracking-[0.04em] text-chalk-faint uppercase transition-colors hover:text-chalk-dim aria-selected:border-grease aria-selected:text-chalk"
              >
                {k.label}
              </button>
            ))}
          </div>
          <div role="tablist" aria-label="Lists" className="-mb-px flex gap-1 overflow-x-auto">
            {lists.map((l) => (
              <button
                key={l.id}
                role="tab"
                aria-selected={l.id === list}
                onClick={() => navigate({ to: "/", search: { list: l.id, kind } })}
                className="shrink-0 border-b-[3px] border-transparent px-3 pb-3 font-slate text-lg font-bold tracking-[0.04em] text-chalk-faint uppercase transition-colors hover:text-chalk-dim aria-selected:border-chalk aria-selected:text-chalk"
              >
                {kind === "show" && l.id === "now_playing" ? "On the air" : l.label}
              </button>
            ))}
          </div>
        </div>
        {isPending ? (
          <RollSkeleton />
        ) : (
          <Roll
            items={items.map((m) => ({
              key: m.tmdb_id,
              tmdbId: m.tmdb_id,
              title: m.title,
              year: m.year,
              posterPath: m.poster_path,
              state: m.local?.state,
              kind,
            }))}
          />
        )}
      </section>
    </main>
  );
}

/** The lead film, full-bleed. The title is the loudest thing on the page. */
function Hero({ film, kind }: { film: CatalogItem; kind: MediaKind }) {
  return (
    <section className="relative isolate flex min-h-[min(78vh,52rem)] items-end overflow-hidden">
      <img
        src={img("w1280", film.backdrop_path)}
        alt=""
        fetchPriority="high"
        className="absolute inset-0 -z-20 size-full object-cover object-[center_25%]"
      />
      {/* scrim so chalk text holds contrast on any backdrop */}
      <div
        aria-hidden
        className="absolute inset-0 -z-10 bg-[linear-gradient(to_top,var(--color-slate)_4%,rgb(20_21_22/0.82)_34%,rgb(20_21_22/0.15)_72%),linear-gradient(to_right,rgb(20_21_22/0.75),transparent_60%)]"
      />
      <div className="mx-auto w-full max-w-[110rem] px-4 pt-32 pb-10 sm:px-8 sm:pb-14">
        <Link
          to={kind === "show" ? "/show/$tmdbId" : "/movie/$tmdbId"}
          params={{ tmdbId: String(film.tmdb_id) }}
          className="block max-w-[16ch] font-slate text-[clamp(3rem,9vw,6rem)] leading-[0.86] font-extrabold tracking-[-0.02em] uppercase hover:text-white"
        >
          {film.title}
        </Link>
        <div className="mt-6 flex max-w-3xl flex-col gap-6 lg:flex-row lg:items-end">
          <SlateFields
            className="min-w-[18rem]"
            fields={[
              { label: "Year", value: film.year, mono: true },
              { label: "Rating", value: film.vote_average ? film.vote_average.toFixed(1) : undefined, mono: true },
            ]}
          />
          <RequestButton tmdbId={film.tmdb_id} local={film.local} kind={kind} />
        </div>
        {film.overview && (
          <p className="mt-6 line-clamp-3 max-w-[60ch] text-[15px] leading-relaxed text-chalk-dim">
            {film.overview}
          </p>
        )}
      </div>
    </section>
  );
}

export function SearchPage({ q }: { q: string }) {
  return (
    <main className="mx-auto max-w-[110rem] px-4 pt-12 sm:px-8">
      <h1 className="mb-10 font-slate text-5xl leading-none font-extrabold uppercase">
        {q ? <>“{q}”</> : "Search"}
      </h1>
      {q ? (
        <>
          {KINDS.map((k) => (
            <section key={k.id} aria-labelledby={`found-${k.id}`} className="mb-14">
              <h2 id={`found-${k.id}`} className="mb-6 border-b border-slate-line pb-3 font-slate text-2xl font-extrabold uppercase">
                {k.label}
              </h2>
              <Found q={q} kind={k.id} />
            </section>
          ))}
          <section aria-labelledby="found-artists" className="mb-14">
            <h2 id="found-artists" className="mb-6 border-b border-slate-line pb-3 font-slate text-2xl font-extrabold uppercase">
              Artists
            </h2>
            <Artists q={q} />
          </section>
        </>
      ) : (
        <p className="text-chalk-dim">Type a title in the search box above.</p>
      )}
    </main>
  );
}

function Found({ q, kind }: { q: string; kind: MediaKind }) {
  const { data, error, isFetching } = useQuery(searchQuery(q, kind));
  if (error instanceof ApiError && error.code === "not_configured") {
    return (
      <p className="text-chalk-dim">
        Add a TMDB key in{" "}
        <Link to="/settings" className="text-grease hover:text-grease-hot">
          Settings
        </Link>{" "}
        to search films and shows.
      </p>
    );
  }
  if (error) return <p className="text-grease-hot">{error.message}</p>;
  const items = data?.results ?? [];
  return (
    <>
      {isFetching && !data ? (
        <RollSkeleton count={12} />
      ) : items.length ? (
        <Roll
          numbered={false}
          items={items.map((m) => ({
            key: m.tmdb_id,
            tmdbId: m.tmdb_id,
            title: m.title,
            year: m.year,
            posterPath: m.poster_path,
            state: m.local?.state,
            kind,
          }))}
        />
      ) : (
        <p className="text-chalk-dim">Nothing matches that. Try the original title or fewer words.</p>
      )}
    </>
  );
}

function Artists({ q }: { q: string }) {
  const { data, error, isFetching } = useQuery(artistSearchQuery(q));
  if (error) return <p className="text-grease-hot">{error.message}</p>;
  if (isFetching && !data) return <div className="h-24 animate-pulse rounded bg-slate-raised/50" />;
  if (!data?.length) return <p className="text-chalk-dim">No artists match that.</p>;
  return (
    <ul className="grid gap-x-8 border-t border-slate-line sm:grid-cols-2">
      {data.map((a) => (
        <li key={a.mbid} className="border-b border-slate-line">
          <Link to="/artist/$mbid" params={{ mbid: a.mbid }} className="flex items-baseline gap-3 py-3 hover:text-white">
            <span className="font-slate text-lg font-semibold">{a.name}</span>
            <span className="truncate text-sm text-chalk-faint">
              {[a.disambiguation, a.kind, a.country].filter(Boolean).join(" · ")}
            </span>
          </Link>
        </li>
      ))}
    </ul>
  );
}

export function CatalogError({ error }: { error: Error }) {
  const notConfigured = error instanceof ApiError && error.code === "not_configured";
  return (
    <main className="mx-auto grid min-h-[60vh] max-w-xl place-content-center px-4 text-center">
      <h1 className="font-slate text-4xl font-extrabold uppercase">
        {notConfigured ? "Connect the catalog" : "The catalog is not answering"}
      </h1>
      <p className="mt-3 text-chalk-dim">
        {notConfigured
          ? "Film and show listings come from TMDB. Add a free TMDB API key and the catalog fills in."
          : error.message}
      </p>
      {notConfigured && (
        <Link to="/settings" className="btn-grease mx-auto mt-8">
          Add TMDB key
        </Link>
      )}
    </main>
  );
}
