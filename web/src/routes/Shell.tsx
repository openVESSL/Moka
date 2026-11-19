import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, Outlet, useNavigate, useRouterState } from "@tanstack/react-router";
import { LogOut, Search } from "lucide-react";
import { useEffect, useState, type FormEvent } from "react";
import { api } from "../api/client";
import { meQuery, titlesQuery } from "../api/queries";
import { Tape } from "../components/Tape";
import { Wordmark } from "../components/Wordmark";
import { STATE_LABEL, isTitleState } from "../lib/states";

const ADMIN = 1;
const MANAGE_REQUESTS = 2;

/** Download progress pushed by the server, keyed by title id. */
type Progress = Record<number, { progress: number; eta?: number | null }>;

export function Shell() {
  const { data: me } = useQuery(meQuery);
  const canManage = !!me && (me.permissions & (ADMIN | MANAGE_REQUESTS)) !== 0;
  const progress = useLiveEvents();

  const nav = [
    { to: "/", label: "Discover" },
    { to: "/library", label: "Library" },
    { to: "/requests", label: canManage ? "Requests" : "My requests" },
    ...(me && me.permissions & ADMIN ? [{ to: "/settings", label: "Settings" }] : []),
  ] as const;

  return (
    <div className="min-h-dvh">
      <header className="sticky top-0 z-40 border-b border-slate-line/70 bg-slate/85 backdrop-blur-md">
        <div className="mx-auto flex h-16 max-w-[110rem] items-center gap-6 px-4 sm:px-8">
          <Link to="/" aria-label="Home" className="shrink-0">
            <Wordmark />
          </Link>
          <nav aria-label="Main" className="hidden gap-1 md:flex">
            {nav.map((n) => (
              <Link
                key={n.to}
                to={n.to}
                activeOptions={{ exact: n.to === "/" }}
                className="relative px-3 py-2 font-slate text-[15px] font-semibold tracking-[0.06em] text-chalk-dim uppercase transition-colors hover:text-chalk data-[status=active]:text-chalk data-[status=active]:after:absolute data-[status=active]:after:inset-x-3 data-[status=active]:after:-bottom-[13px] data-[status=active]:after:h-[3px] data-[status=active]:after:bg-chalk"
              >
                {n.label}
              </Link>
            ))}
          </nav>
          <SearchBox />
          <SignOut name={me?.username} />
        </div>
        <nav aria-label="Main" className="flex gap-1 overflow-x-auto border-t border-slate-line/60 px-2 md:hidden">
          {nav.map((n) => (
            <Link
              key={n.to}
              to={n.to}
              activeOptions={{ exact: n.to === "/" }}
              className="shrink-0 px-3 py-2.5 font-slate text-sm font-semibold tracking-[0.06em] text-chalk-dim uppercase data-[status=active]:text-chalk"
            >
              {n.label}
            </Link>
          ))}
        </nav>
        <StatusBand progress={progress} />
      </header>
      <Outlet />
      <footer className="mx-auto mt-24 max-w-[110rem] px-4 pb-10 text-xs text-chalk-faint sm:px-8">
        Film data and images from TMDB. This product uses the TMDB API but is not endorsed or certified by TMDB.
      </footer>
    </div>
  );
}

/** One line under the header while anything is moving through the pipeline. */
function StatusBand({ progress }: { progress: Progress }) {
  const { data: titles } = useQuery({ ...titlesQuery, refetchInterval: 30_000 });
  const moving = (titles ?? []).filter((t) =>
    ["searching", "grabbed", "downloading", "importing"].includes(t.state),
  );
  if (!moving.length) return null;
  const first = moving[0];
  const p = progress[first.id];
  return (
    <div className="border-t border-slate-line/60 bg-slate-sunk/80">
      <Link
        to="/library"
        className="mx-auto flex h-9 max-w-[110rem] items-center gap-3 px-4 text-sm sm:px-8"
      >
        <Tape state={first.state} />
        <span className="truncate font-slate font-semibold">{first.title}</span>
        <span className="text-chalk-dim">
          {isTitleState(first.state) ? STATE_LABEL[first.state].toLowerCase() : first.state}
          {p ? <span className="data"> · {Math.round(p.progress * 100)}%</span> : null}
        </span>
        {moving.length > 1 && (
          <span className="ml-auto shrink-0 data text-xs text-chalk-faint">+{moving.length - 1} more</span>
        )}
      </Link>
    </div>
  );
}

function SearchBox() {
  const navigate = useNavigate();
  const current = useRouterState({ select: (s) => (s.location.search as { q?: string }).q ?? "" });
  const [q, setQ] = useState(current);
  useEffect(() => setQ(current), [current]);

  function submit(e: FormEvent) {
    e.preventDefault();
    if (q.trim()) navigate({ to: "/search", search: { q: q.trim() } });
  }

  return (
    <form role="search" onSubmit={submit} className="ml-auto flex min-w-0 items-center">
      <label className="flex min-w-0 items-center gap-2 border-b border-slate-line px-1 focus-within:border-grease">
        <Search aria-hidden className="size-4 shrink-0 text-chalk-faint" strokeWidth={2} />
        <span className="sr-only">Search films</span>
        <input
          type="search"
          value={q}
          onChange={(e) => setQ(e.target.value)}
          placeholder="Search films"
          className="h-9 w-36 min-w-0 bg-transparent text-[15px] placeholder:text-chalk-faint focus:outline-none sm:w-56"
        />
      </label>
    </form>
  );
}

function SignOut({ name }: { name?: string }) {
  const qc = useQueryClient();
  const navigate = useNavigate();
  return (
    <button
      type="button"
      onClick={async () => {
        await api.POST("/api/v1/auth/logout");
        qc.clear();
        navigate({ to: "/login" });
      }}
      className="flex shrink-0 items-center gap-2 text-sm text-chalk-dim hover:text-chalk"
      title={name ? `Signed in as ${name}. Sign out` : "Sign out"}
    >
      <span className="hidden lg:inline">{name}</span>
      <LogOut aria-hidden className="size-4" />
      <span className="sr-only">Sign out</span>
    </button>
  );
}

/** Subscribes to server events and refreshes what they touch. */
function useLiveEvents(): Progress {
  const qc = useQueryClient();
  const [progress, setProgress] = useState<Progress>({});
  useEffect(() => {
    const es = new EventSource("/api/v1/events");
    es.onmessage = (msg) => {
      try {
        const ev = JSON.parse(msg.data) as { type: string; title_id?: number; progress?: number; eta_seconds?: number };
        if (ev.type === "download_progress" && ev.title_id !== undefined) {
          setProgress((p) => ({ ...p, [ev.title_id!]: { progress: ev.progress ?? 0, eta: ev.eta_seconds } }));
          return;
        }
        qc.invalidateQueries({ queryKey: ["titles"] });
        qc.invalidateQueries({ queryKey: ["requests"] });
        qc.invalidateQueries({ queryKey: ["catalog"] });
      } catch {
        // ignore malformed events
      }
    };
    return () => es.close();
  }, [qc]);
  return progress;
}
