import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { Play, Search } from "lucide-react";
import { api, unwrap, type Schemas } from "../api/client";
import { STATE_HINT, isTitleState } from "../lib/states";
import { StateBadge } from "./StateBadge";

const MANAGE_LIBRARY = 16;
const ADMIN = 1;

function size(bytes?: number | null) {
  if (!bytes) return "–";
  const gb = bytes / 1024 ** 3;
  return gb >= 1 ? `${gb.toFixed(1)} GB` : `${Math.round(bytes / 1024 ** 2)} MB`;
}

/**
 * Where a film stands locally: its state, what happens next, the file to
 * play, and the last search with the chosen release marked and a reason
 * next to every other one.
 */
export function TitlePanel({ titleId, permissions }: { titleId: number; permissions: number }) {
  const qc = useQueryClient();
  const canManage = (permissions & (ADMIN | MANAGE_LIBRARY)) !== 0;
  const { data: title } = useQuery({
    queryKey: ["titles", titleId],
    queryFn: () => unwrap(api.GET("/api/v1/titles/{id}", { params: { path: { id: titleId } } })),
    refetchInterval: 15_000,
  });
  const { data: files } = useQuery({
    queryKey: ["titles", titleId, "files"],
    queryFn: () => unwrap(api.GET("/api/v1/titles/{id}/files", { params: { path: { id: titleId } } })),
  });
  const { data: last } = useQuery({
    queryKey: ["titles", titleId, "releases"],
    queryFn: () => unwrap(api.GET("/api/v1/titles/{id}/releases", { params: { path: { id: titleId } } })),
  });
  const search = useMutation({
    mutationFn: () => unwrap(api.POST("/api/v1/titles/{id}/search", { params: { path: { id: titleId } } })),
    onSuccess: (v) => qc.setQueryData(["titles", titleId, "releases"], v),
  });
  const grab = useMutation({
    mutationFn: (index: number) =>
      unwrap(api.POST("/api/v1/titles/{id}/grab", { params: { path: { id: titleId } }, body: { index } })),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["titles"] }),
  });

  if (!title) return null;
  const file = files?.[0];
  const releases = (search.data ?? last)?.releases ?? [];

  return (
    <section aria-labelledby="report" className="mx-auto mt-8 max-w-[110rem] px-4 sm:px-8">
      <div className="flex flex-wrap items-center gap-x-5 gap-y-3 border-y border-line py-5">
        <h2 id="report" className="sr-only">
          Status
        </h2>
        <StateBadge state={title.state} />
        <p className="min-w-0 flex-1 text-muted">
          {title.state_reason ?? (isTitleState(title.state) ? STATE_HINT[title.state] : "")}
        </p>
        {file && (
          <Link to="/watch/$fileId" params={{ fileId: String(file.id) }} className="btn-primary">
            <Play aria-hidden className="size-4" fill="currentColor" /> Play
          </Link>
        )}
        {canManage && (
          <button type="button" className="btn" disabled={search.isPending} onClick={() => search.mutate()}>
            <Search aria-hidden className="size-4" />
            {search.isPending ? "Searching…" : "Search releases"}
          </button>
        )}
      </div>

      {file && (
        <p className="mt-3 data text-sm text-faint">
          {file.quality_label} · {size(file.size_bytes)}
          {file.release_group ? ` · ${file.release_group}` : ""}
        </p>
      )}
      {search.error && <p className="mt-3 text-sm text-danger">{search.error.message}</p>}
      {grab.error && <p className="mt-3 text-sm text-danger">{grab.error.message}</p>}

      {releases.length > 0 && (
        <ReleaseReport releases={releases} canGrab={canManage} grabbing={grab.isPending} onGrab={grab.mutate} />
      )}
      {(search.data ?? last)?.indexer_errors.map((e) => (
        <p key={e} className="mt-2 text-sm text-danger">
          {e}
        </p>
      ))}
    </section>
  );
}

function ReleaseReport({
  releases,
  canGrab,
  grabbing,
  onGrab,
}: {
  releases: Schemas["ReleaseView"][];
  canGrab: boolean;
  grabbing: boolean;
  onGrab: (index: number) => void;
}) {
  return (
    <div className="mt-6 overflow-x-auto">
      <table className="w-full min-w-[46rem] border-collapse text-left text-sm">
        <thead>
          <tr className="field-label border-b border-line text-[10px]">
            <th scope="col" className="w-12 py-2 font-semibold">Take</th>
            <th scope="col" className="py-2 font-semibold">Release</th>
            <th scope="col" className="py-2 font-semibold">Quality</th>
            <th scope="col" className="py-2 text-right font-semibold">Size</th>
            <th scope="col" className="py-2 text-right font-semibold">Seeds</th>
            <th scope="col" className="py-2 pl-4 font-semibold">Notes</th>
            {canGrab && <th scope="col" className="py-2"><span className="sr-only">Grab</span></th>}
          </tr>
        </thead>
        <tbody>
          {releases.map((r, i) => (
            <tr key={r.index} className={`border-b border-line/60 ${r.accepted ? "" : "text-faint"}`}>
              <td className="py-2.5">
                <span className="relative data px-1.5">
                  {String(i + 1).padStart(2, "0")}
                  {i === 0 && r.accepted && <span className="ml-1 text-accent">best</span>}
                </span>
              </td>
              <td className="max-w-[28rem] py-2.5 pr-4">
                <span className={`block truncate data text-[13px] ${r.accepted ? "text-fg" : ""}`} title={r.title}>
                  {r.title}
                </span>
                <span className="text-xs text-faint">{r.indexer}</span>
              </td>
              <td className="py-2.5 pr-4 whitespace-nowrap">{r.quality_label ?? "–"}</td>
              <td className="py-2.5 text-right data whitespace-nowrap">{size(r.size_bytes)}</td>
              <td className="py-2.5 text-right data">{r.seeders ?? "–"}</td>
              <td className="py-2.5 pl-4">
                {r.accepted ? (
                  r.beats_next_by ? (
                    <span className="text-state-available">Ahead on {r.beats_next_by}</span>
                  ) : (
                    <span className="text-state-available">Acceptable</span>
                  )
                ) : (
                  r.rejections.join(" · ")
                )}
              </td>
              {canGrab && (
                <td className="py-2.5 pl-3 text-right">
                  <button
                    type="button"
                    className="text-xs font-bold text-accent hover:text-accent-hot disabled:opacity-50"
                    disabled={grabbing}
                    onClick={() => onGrab(r.index)}
                  >
                    Grab
                  </button>
                </td>
              )}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
