import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { img } from "../api/client";
import { meQuery, requestsQuery, useDecideRequest } from "../api/queries";
import { Tape } from "../components/Tape";
import { ago } from "../lib/states";

const FILTERS = [
  { id: "pending", label: "Waiting" },
  { id: "approved", label: "Approved" },
  { id: "declined", label: "Declined" },
] as const;
type Filter = (typeof FILTERS)[number]["id"];

/** Requests laid out like a camera report: one ruled line per film. */
export function RequestsPage({ filter }: { filter: Filter }) {
  const { data: me } = useQuery(meQuery);
  const canManage = !!me && (me.permissions & 3) !== 0;
  const { data, isPending } = useQuery(requestsQuery(filter));
  const decide = useDecideRequest();
  const rows = data ?? [];

  return (
    <main className="mx-auto max-w-[80rem] px-4 pt-12 sm:px-8">
      <h1 className="font-slate text-5xl leading-none font-extrabold uppercase">
        {canManage ? "Requests" : "My requests"}
      </h1>
      <div role="tablist" aria-label="Filter" className="mt-8 flex gap-1 border-b border-slate-line">
        {FILTERS.map((f) => (
          <Link
            key={f.id}
            to="/requests"
            search={{ status: f.id }}
            role="tab"
            aria-selected={f.id === filter}
            className="-mb-px border-b-[3px] border-transparent px-3 pb-3 font-slate text-lg font-bold tracking-[0.04em] text-chalk-faint uppercase hover:text-chalk-dim aria-selected:border-chalk aria-selected:text-chalk"
          >
            {f.label}
          </Link>
        ))}
      </div>

      {isPending ? (
        <div className="mt-6 h-40 animate-pulse rounded bg-slate-raised/50" />
      ) : rows.length === 0 ? (
        <p className="mt-10 max-w-[50ch] text-chalk-dim">
          {filter === "pending"
            ? canManage
              ? "Nothing waiting for a decision."
              : "Nothing waiting. Find a film on Discover and press Request."
            : "No requests here yet."}
        </p>
      ) : (
        <table className="mt-2 w-full border-collapse text-left">
          <thead>
            <tr className="slate-label border-b border-slate-line text-[10px]">
              <th scope="col" className="py-3 pr-4 font-semibold">Film</th>
              {canManage && <th scope="col" className="hidden py-3 pr-4 font-semibold sm:table-cell">Asked by</th>}
              <th scope="col" className="hidden py-3 pr-4 font-semibold md:table-cell">When</th>
              <th scope="col" className="py-3 pr-4 font-semibold">State</th>
              {canManage && filter === "pending" && <th scope="col" className="py-3 text-right font-semibold"><span className="sr-only">Decision</span></th>}
            </tr>
          </thead>
          <tbody>
            {rows.map((r) => (
              <tr key={r.id} className="border-b border-slate-line/70">
                <td className="py-3 pr-4">
                  <Link
                    to="/movie/$tmdbId"
                    params={{ tmdbId: String(r.tmdb_id ?? 0) }}
                    className="flex items-center gap-4 hover:text-white"
                  >
                    {r.poster_path ? (
                      <img src={img("w185", r.poster_path)} alt="" className="h-16 w-11 shrink-0 rounded-[2px] object-cover" />
                    ) : (
                      <span className="h-16 w-11 shrink-0 rounded-[2px] bg-slate-raised" />
                    )}
                    <span>
                      <span className="block font-slate text-lg leading-tight font-bold">{r.title}</span>
                      {r.year && <span className="data text-xs text-chalk-faint">{r.year}</span>}
                    </span>
                  </Link>
                </td>
                {canManage && <td className="hidden py-3 pr-4 text-chalk-dim sm:table-cell">{r.requested_by}</td>}
                <td className="hidden py-3 pr-4 data text-sm text-chalk-faint md:table-cell">{ago(r.created_at)}</td>
                <td className="py-3 pr-4">
                  <Tape state={r.title_state} className="-rotate-2" />
                  {r.note && <p className="mt-1 text-xs text-chalk-faint">{r.note}</p>}
                </td>
                {canManage && filter === "pending" && (
                  <td className="py-3 text-right">
                    <div className="inline-flex flex-wrap justify-end gap-2">
                      <button
                        type="button"
                        className="btn-grease min-h-9 px-3 text-sm"
                        disabled={decide.isPending}
                        onClick={() => decide.mutate({ id: r.id, approve: true })}
                      >
                        Approve
                      </button>
                      <button
                        type="button"
                        className="btn-chalk min-h-9 px-3 text-sm"
                        disabled={decide.isPending}
                        onClick={() => decide.mutate({ id: r.id, approve: false })}
                      >
                        Decline
                      </button>
                    </div>
                  </td>
                )}
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {decide.error && (
        <p role="alert" className="mt-4 text-sm text-grease-hot">
          {decide.error.message}
        </p>
      )}
    </main>
  );
}
