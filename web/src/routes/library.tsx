import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { titlesQuery } from "../api/queries";
import { Roll, RollSkeleton } from "../components/Roll";

const GROUPS = [
  { title: "In your library", states: ["available"] },
  { title: "On the way", states: ["searching", "grabbed", "downloading", "importing"] },
  { title: "Wanted", states: ["wanted"] },
  { title: "Waiting for approval", states: ["requested"] },
] as const;

export function LibraryPage() {
  const { data, isPending } = useQuery(titlesQuery);
  const titles = data ?? [];

  return (
    <main className="mx-auto max-w-[110rem] px-4 pt-12 sm:px-8">
      <h1 className="font-slate text-5xl leading-none font-extrabold uppercase">Library</h1>
      {isPending ? (
        <div className="mt-10">
          <RollSkeleton count={12} />
        </div>
      ) : titles.length === 0 ? (
        <div className="mt-10 max-w-[52ch]">
          <p className="text-lg text-chalk-dim">
            Nothing here yet. Films you request show up on this page as they move from wanted to ready.
          </p>
          <Link to="/" className="btn-grease mt-6">
            Find a film
          </Link>
        </div>
      ) : (
        GROUPS.map((g) => {
          const items = titles.filter((t) => (g.states as readonly string[]).includes(t.state));
          if (!items.length) return null;
          return (
            <section key={g.title} className="mt-14" aria-labelledby={`g-${g.title}`}>
              <h2
                id={`g-${g.title}`}
                className="mb-6 flex items-baseline gap-3 border-b border-slate-line pb-3 font-slate text-2xl font-extrabold uppercase"
              >
                {g.title}
                <span className="data text-sm font-normal text-chalk-faint">{items.length}</span>
              </h2>
              <Roll
                numbered={false}
                items={items.map((t) => ({
                  key: t.id,
                  tmdbId: t.tmdb_id,
                  title: t.title,
                  year: t.year,
                  posterPath: t.poster_path,
                  state: t.state,
                }))}
              />
            </section>
          );
        })
      )}
    </main>
  );
}
