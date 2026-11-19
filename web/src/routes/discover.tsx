import { useQuery } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { ApiError, img, type CatalogItem } from "../api/client";
import { DISCOVER_LISTS, discoverQuery, searchQuery, type DiscoverList } from "../api/queries";
import { RequestButton } from "../components/RequestButton";
import { Roll, RollSkeleton } from "../components/Roll";
import { SlateFields } from "../components/SlateFields";

export function DiscoverPage({ list }: { list: DiscoverList }) {
  const { data, error, isPending } = useQuery(discoverQuery(list));
  const navigate = useNavigate();

  if (error) return <CatalogError error={error} />;
  const items = data?.results ?? [];
  const lead = items.find((m) => m.backdrop_path) ?? items[0];

  return (
    <main>
      {lead ? <Hero film={lead} /> : <div className="h-[62vh] animate-pulse bg-slate-raised/50" />}

      <section aria-labelledby="roll-heading" className="mx-auto max-w-[110rem] px-4 sm:px-8">
        <div className="mt-10 mb-8 flex flex-wrap items-end gap-x-8 gap-y-4 border-b border-slate-line">
          <h2 id="roll-heading" className="sr-only">
            {DISCOVER_LISTS.find((l) => l.id === list)?.label}
          </h2>
          <div role="tablist" aria-label="Lists" className="-mb-px flex gap-1 overflow-x-auto">
            {DISCOVER_LISTS.map((l) => (
              <button
                key={l.id}
                role="tab"
                aria-selected={l.id === list}
                onClick={() => navigate({ to: "/", search: { list: l.id } })}
                className="shrink-0 border-b-[3px] border-transparent px-3 pb-3 font-slate text-lg font-bold tracking-[0.04em] text-chalk-faint uppercase transition-colors hover:text-chalk-dim aria-selected:border-chalk aria-selected:text-chalk"
              >
                {l.label}
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
            }))}
          />
        )}
      </section>
    </main>
  );
}

/** The lead film, full-bleed. The title is the loudest thing on the page. */
function Hero({ film }: { film: CatalogItem }) {
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
          to="/movie/$tmdbId"
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
          <RequestButton tmdbId={film.tmdb_id} local={film.local} />
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
  const { data, error, isFetching } = useQuery(searchQuery(q));
  if (error) return <CatalogError error={error} />;
  const items = data?.results ?? [];
  return (
    <main className="mx-auto max-w-[110rem] px-4 pt-12 sm:px-8">
      <h1 className="mb-10 font-slate text-5xl leading-none font-extrabold uppercase">
        {q ? <>“{q}”</> : "Search"}
      </h1>
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
          }))}
        />
      ) : q ? (
        <p className="text-chalk-dim">No films match that. Try the original title or fewer words.</p>
      ) : (
        <p className="text-chalk-dim">Type a title in the search box above.</p>
      )}
    </main>
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
          ? "Film listings come from TMDB. Add a free TMDB API key and the catalog fills in."
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
