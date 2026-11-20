import { useQuery } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { ChevronRight, Info, Search, Star } from "lucide-react";
import { useEffect, useState, type FormEvent } from "react";
import { ApiError, img, type CatalogItem } from "../api/client";
import {
  DISCOVER_LISTS,
  SHOW_LISTS,
  discoverQuery,
  artistSearchQuery,
  bookSearchQuery,
  searchQuery,
  type DiscoverList,
  type MediaKind,
} from "../api/queries";
import { RequestButton } from "../components/RequestButton";
import { PosterRow, PosterGrid, PosterGridSkeleton, type PosterItem } from "../components/Posters";

const KINDS: { id: MediaKind; label: string }[] = [
  { id: "movie", label: "Films" },
  { id: "show", label: "Shows" },
];

const PILL =
  "shrink-0 rounded-full px-3.5 py-1.5 text-sm font-medium text-muted transition-colors hover:text-fg aria-selected:bg-white/10 aria-selected:text-fg";

const listLabel = (id: DiscoverList, kind: MediaKind) =>
  kind === "show" && id === "now_playing" ? "On the air" : DISCOVER_LISTS.find((l) => l.id === id)!.label;

const toItems = (results: CatalogItem[], kind: MediaKind): PosterItem[] =>
  results.map((m) => ({
    key: m.tmdb_id,
    tmdbId: m.tmdb_id,
    title: m.title,
    year: m.year,
    posterPath: m.poster_path,
    state: m.local?.state,
    kind,
  }));

/** Home: a lead title on top, then one row per list. With `list` set, that list as a full grid. */
export function DiscoverPage({ list, kind }: { list?: DiscoverList; kind: MediaKind }) {
  const { data, error } = useQuery(discoverQuery(list ?? "trending", 1, kind));
  const navigate = useNavigate();

  if (error) return <CatalogError error={error} />;
  const lists = DISCOVER_LISTS.filter((l) => kind === "movie" || SHOW_LISTS.includes(l.id));

  const kindSwitch = (
    <div role="tablist" aria-label="Films or shows" className="flex gap-1 rounded-full bg-black/40 p-1 backdrop-blur-md">
      {KINDS.map((k) => (
        <button
          key={k.id}
          role="tab"
          aria-selected={k.id === kind}
          onClick={() => navigate({ to: "/", search: { ...(list ? { list } : {}), kind: k.id } })}
          className={PILL}
        >
          {k.label}
        </button>
      ))}
    </div>
  );

  if (list) {
    return (
      <main className="mx-auto max-w-[110rem] px-4 pt-8 pb-20 sm:px-8">
        <div className="mb-8 flex flex-wrap items-center gap-4">
          {kindSwitch}
          <div role="tablist" aria-label="Lists" className="no-scrollbar flex gap-1 overflow-x-auto">
            {lists.map((l) => (
              <button
                key={l.id}
                role="tab"
                aria-selected={l.id === list}
                onClick={() => navigate({ to: "/", search: { list: l.id, kind } })}
                className={PILL}
              >
                {listLabel(l.id, kind)}
              </button>
            ))}
          </div>
        </div>
        <h1 className="sr-only">{listLabel(list, kind)}</h1>
        {data ? <PosterGrid items={toItems(data.results, kind)} /> : <PosterGridSkeleton />}
      </main>
    );
  }

  const lead = data?.results.find((m) => m.backdrop_path) ?? data?.results[0];
  return (
    <main className="pb-20">
      <div className="relative">
        {lead ? <Hero film={lead} kind={kind} /> : <div className="h-[min(72vh,44rem)] animate-pulse bg-raised/40" />}
        <div className="absolute top-5 left-4 sm:left-8">{kindSwitch}</div>
      </div>
      <div className="relative z-10 -mt-16 space-y-10 px-4 sm:px-8">
        {lists.map((l) => (
          <ListRow key={l.id} list={l.id} kind={kind} />
        ))}
      </div>
    </main>
  );
}

function ListRow({ list, kind }: { list: DiscoverList; kind: MediaKind }) {
  const { data } = useQuery(discoverQuery(list, 1, kind));
  return (
    <PosterRow
      title={listLabel(list, kind)}
      items={data ? toItems(data.results, kind) : undefined}
      action={
        <Link
          to="/"
          search={{ list, kind }}
          className="inline-flex items-center gap-0.5 text-sm font-medium text-accent hover:text-accent-hot"
        >
          See all <ChevronRight aria-hidden className="size-4" />
        </Link>
      }
    />
  );
}

/** The lead title, edge to edge, fading into the page. */
function Hero({ film, kind }: { film: CatalogItem; kind: MediaKind }) {
  const to = kind === "show" ? "/show/$tmdbId" : "/movie/$tmdbId";
  return (
    <section className="relative isolate flex min-h-[min(72vh,44rem)] items-end overflow-hidden">
      <img
        src={img("w1280", film.backdrop_path)}
        alt=""
        fetchPriority="high"
        className="absolute inset-0 -z-20 size-full object-cover object-[center_20%]"
      />
      {/* scrim so text holds contrast on any backdrop, and the image melts into the page */}
      <div
        aria-hidden
        className="absolute inset-0 -z-10 bg-[linear-gradient(to_top,var(--color-canvas)_2%,rgb(12_12_14/0.7)_30%,transparent_65%),linear-gradient(to_right,rgb(12_12_14/0.85),rgb(12_12_14/0.3)_45%,transparent_70%)]"
      />
      <div className="w-full px-4 pb-28 sm:px-8">
        <p className="text-sm font-medium text-accent">#1 trending {kind === "show" ? "show" : "film"}</p>
        <Link
          to={to}
          params={{ tmdbId: String(film.tmdb_id) }}
          className="mt-2 block max-w-[20ch] text-[clamp(2rem,4.5vw,3.75rem)] leading-[1.02] font-bold tracking-[-0.03em] hover:text-white"
        >
          {film.title}
        </Link>
        <div className="mt-3 flex items-center gap-3 text-sm text-muted">
          {film.year && <span className="data">{film.year}</span>}
          {film.vote_average ? (
            <span className="inline-flex items-center gap-1">
              <Star aria-hidden className="size-3.5 fill-current text-state-requested" />
              <span className="data">{film.vote_average.toFixed(1)}</span>
            </span>
          ) : null}
        </div>
        {film.overview && (
          <p className="mt-3 line-clamp-3 max-w-[56ch] leading-relaxed text-fg/80">{film.overview}</p>
        )}
        <div className="mt-6 flex flex-wrap items-center gap-3">
          <RequestButton tmdbId={film.tmdb_id} local={film.local} kind={kind} />
          <Link to={to} params={{ tmdbId: String(film.tmdb_id) }} className="btn">
            <Info aria-hidden className="size-4" /> More info
          </Link>
        </div>
      </div>
    </section>
  );
}

export function SearchPage({ q }: { q: string }) {
  const navigate = useNavigate();
  const [text, setText] = useState(q);
  useEffect(() => setText(q), [q]);

  function submit(e: FormEvent) {
    e.preventDefault();
    navigate({ to: "/search", search: { q: text.trim() } });
  }

  return (
    <main className="mx-auto max-w-[110rem] px-4 pt-10 pb-20 sm:px-8">
      <form role="search" onSubmit={submit} className="max-w-xl">
        <label className="flex h-12 items-center gap-3 rounded-xl border border-line bg-white/[0.03] px-4 transition-colors focus-within:border-accent">
          <Search aria-hidden className="size-4 shrink-0 text-faint" />
          <span className="sr-only">Search films, shows, music and books</span>
          <input
            type="search"
            autoFocus
            value={text}
            onChange={(e) => setText(e.target.value)}
            placeholder="Search films, shows, artists, books"
            className="h-full min-w-0 flex-1 bg-transparent placeholder:text-faint focus:outline-none"
          />
        </label>
      </form>
      {q ? (
        <>
          {KINDS.map((k) => (
            <section key={k.id} aria-labelledby={`found-${k.id}`} className="mt-8">
              <h2 id={`found-${k.id}`} className="mb-3 text-base font-semibold">
                {k.label}
              </h2>
              <Found q={q} kind={k.id} />
            </section>
          ))}
          <section aria-labelledby="found-artists" className="mt-8">
            <h2 id="found-artists" className="mb-3 text-base font-semibold">
              Artists
            </h2>
            <Artists q={q} />
          </section>
          <section aria-labelledby="found-books" className="mt-8">
            <h2 id="found-books" className="mb-3 text-base font-semibold">
              Books
            </h2>
            <Books q={q} />
          </section>
        </>
      ) : (
        <p className="mt-4 text-sm text-muted">Type a title, an artist or an author and press Enter.</p>
      )}
    </main>
  );
}


function Found({ q, kind }: { q: string; kind: MediaKind }) {
  const { data, error, isFetching } = useQuery(searchQuery(q, kind));
  if (error instanceof ApiError && error.code === "not_configured") {
    return (
      <p className="text-muted">
        Add a TMDB key in{" "}
        <Link to="/settings" className="text-accent hover:text-accent-hot">
          Settings
        </Link>{" "}
        to search films and shows.
      </p>
    );
  }
  if (error) return <p className="text-danger">{error.message}</p>;
  const items = data?.results ?? [];
  return (
    <>
      {isFetching && !data ? (
        <PosterGridSkeleton count={12} />
      ) : items.length ? (
        <PosterGrid
         
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
        <p className="text-muted">Nothing matches that. Try the original title or fewer words.</p>
      )}
    </>
  );
}

function Artists({ q }: { q: string }) {
  const { data, error, isFetching } = useQuery(artistSearchQuery(q));
  if (error) return <p className="text-danger">{error.message}</p>;
  if (isFetching && !data) return <div className="h-24 animate-pulse rounded bg-raised/50" />;
  if (!data?.length) return <p className="text-muted">No artists match that.</p>;
  return (
    <ul className="grid gap-x-8 border-t border-line sm:grid-cols-2">
      {data.map((a) => (
        <li key={a.mbid} className="border-b border-line">
          <Link to="/artist/$mbid" params={{ mbid: a.mbid }} className="flex items-baseline gap-3 py-3 hover:text-white">
            <span className="text-lg font-semibold">{a.name}</span>
            <span className="truncate text-sm text-faint">
              {[a.disambiguation, a.kind, a.country].filter(Boolean).join(" · ")}
            </span>
          </Link>
        </li>
      ))}
    </ul>
  );
}

function Books({ q }: { q: string }) {
  const { data, error, isFetching } = useQuery(bookSearchQuery(q));
  if (error) return <p className="text-danger">{error.message}</p>;
  if (isFetching && !data) return <PosterGridSkeleton count={12} />;
  if (!data?.length) return <p className="text-muted">No books match that.</p>;
  return (
    <PosterGrid
      items={data.map((b) => ({
        key: b.olid,
        title: b.title,
        year: b.year,
        kind: "book" as const,
        olid: b.olid,
        coverId: b.cover_id,
        note: b.authors[0],
        state: b.ebook?.state ?? b.audiobook?.state,
      }))}
    />
  );
}

export function CatalogError({ error }: { error: Error }) {
  const notConfigured = error instanceof ApiError && error.code === "not_configured";
  return (
    <main className="mx-auto grid min-h-[60vh] max-w-xl place-content-center px-4 text-center">
      <h1 className="text-2xl font-semibold">
        {notConfigured ? "Connect the catalog" : "The catalog is not answering"}
      </h1>
      <p className="mt-3 text-muted">
        {notConfigured
          ? "Film and show listings come from TMDB. Add a free TMDB API key and the catalog fills in."
          : error.message}
      </p>
      {notConfigured && (
        <Link to="/settings" className="btn-primary mx-auto mt-8">
          Add TMDB key
        </Link>
      )}
    </main>
  );
}
