import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, Outlet, useNavigate, useRouterState } from "@tanstack/react-router";
import { House, Inbox, Info, Library, LogOut, Menu, Plug, Search, Settings, X, type LucideIcon } from "lucide-react";
import { useEffect, useState } from "react";
import { api } from "../api/client";
import { meQuery, titlesQuery } from "../api/queries";
import { Tape } from "../components/Tape";
import { Wordmark } from "../components/Wordmark";

const ADMIN = 1;
const MANAGE_REQUESTS = 2;

/** Download progress pushed by the server, keyed by title id. */
type Progress = Record<number, { progress: number; eta?: number | null }>;

type NavItem = { to: string; label: string; icon: LucideIcon };

export function Shell() {
  const { data: me } = useQuery(meQuery);
  const isAdmin = !!me && (me.permissions & ADMIN) !== 0;
  const canManage = !!me && (me.permissions & (ADMIN | MANAGE_REQUESTS)) !== 0;
  const progress = useLiveEvents();
  const [open, setOpen] = useState(false);
  const path = useRouterState({ select: (s) => s.location.pathname });
  useEffect(() => setOpen(false), [path]);

  const main: NavItem[] = [
    { to: "/", label: "Home", icon: House },
    { to: "/search", label: "Search", icon: Search },
    { to: "/library", label: "Library", icon: Library },
    { to: "/requests", label: canManage ? "Requests" : "My requests", icon: Inbox },
  ];
  const manage: NavItem[] = isAdmin
    ? [
        { to: "/connections", label: "Connections", icon: Plug },
        { to: "/settings", label: "Settings", icon: Settings },
      ]
    : [];

  return (
    <div className="min-h-dvh">
      <header className="sticky top-0 z-30 flex h-14 items-center gap-3 border-b border-line bg-canvas/80 px-3 backdrop-blur-xl md:hidden">
        <button
          type="button"
          onClick={() => setOpen(true)}
          className="grid size-10 place-items-center rounded-lg text-muted hover:bg-white/5 hover:text-fg"
          aria-label="Open menu"
        >
          <Menu aria-hidden className="size-5" />
        </button>
        <Link to="/" aria-label="Home">
          <Wordmark />
        </Link>
      </header>

      {open && (
        <div aria-hidden className="fixed inset-0 z-40 bg-black/60 backdrop-blur-sm md:hidden" onClick={() => setOpen(false)} />
      )}

      <aside
        className={`fixed inset-y-0 left-0 z-50 flex w-64 flex-col border-r border-white/[0.06] bg-sunk/95 backdrop-blur-xl transition-transform duration-300 ease-[var(--ease-out-expo)] md:translate-x-0 ${open ? "translate-x-0" : "-translate-x-full"}`}
      >
        <div className="flex h-20 items-center justify-between px-6">
          <Link to="/" aria-label="Home">
            <Wordmark />
          </Link>
          <button
            type="button"
            onClick={() => setOpen(false)}
            className="grid size-9 place-items-center rounded-lg text-muted hover:text-fg md:hidden"
            aria-label="Close menu"
          >
            <X aria-hidden className="size-4" />
          </button>
        </div>

        <nav aria-label="Main" className="flex-1 overflow-y-auto px-3">
          <NavList items={main} />
          {manage.length > 0 && (
            <>
              <p className="mt-7 mb-2 px-3 text-xs font-medium text-faint">Manage</p>
              <NavList items={manage} />
            </>
          )}
        </nav>

        <div className="px-3 pb-2">
          <Activity progress={progress} />
          <NavList items={[{ to: "/about", label: "About", icon: Info }]} />
        </div>
        <Profile name={me?.username} role={isAdmin ? "Admin" : canManage ? "Manager" : "Member"} />
      </aside>

      <div className="md:pl-64">
        <Outlet />
      </div>
    </div>
  );
}

function NavList({ items }: { items: NavItem[] }) {
  return (
    <ul className="space-y-1">
      {items.map((n) => (
        <li key={n.to}>
          <Link
            to={n.to}
            activeOptions={{ exact: n.to === "/" }}
            className="group flex h-10 items-center gap-3 rounded-lg px-3 text-[0.95rem] font-medium text-muted transition-colors hover:bg-white/[0.04] hover:text-fg data-[status=active]:bg-[linear-gradient(90deg,rgb(91_94_240/0.24),rgb(91_94_240/0.06))] data-[status=active]:text-fg"
          >
            <n.icon
              aria-hidden
              className="size-[18px] shrink-0 transition-colors group-data-[status=active]:text-accent"
              strokeWidth={1.9}
            />
            {n.label}
          </Link>
        </li>
      ))}
    </ul>
  );
}

/** What is moving through the pipeline right now, above the profile. */
function Activity({ progress }: { progress: Progress }) {
  const { data: titles } = useQuery({ ...titlesQuery, refetchInterval: 30_000 });
  const moving = (titles ?? []).filter((t) =>
    ["searching", "grabbed", "downloading", "importing"].includes(t.state),
  );
  if (!moving.length) return null;
  const first = moving[0];
  const p = progress[first.id];
  return (
    <Link
      to="/library"
      className="mb-2 block rounded-xl border border-line bg-white/[0.03] px-3.5 py-3 text-sm transition-colors hover:border-line-strong"
    >
      <div className="flex items-center justify-between gap-2">
        <Tape state={first.state} />
        {p && <span className="data text-xs text-muted">{Math.round(p.progress * 100)}%</span>}
      </div>
      <p className="mt-1.5 truncate font-medium">{first.title}</p>
      {p && (
        <div className="mt-1.5 h-1 overflow-hidden rounded-full bg-line">
          <div className="h-full bg-accent" style={{ width: `${Math.round(p.progress * 100)}%` }} />
        </div>
      )}
      {moving.length > 1 && <p className="mt-1 text-xs text-faint">+{moving.length - 1} more</p>}
    </Link>
  );
}

function Profile({ name, role }: { name?: string; role: string }) {
  const qc = useQueryClient();
  const navigate = useNavigate();
  return (
    <div className="flex items-center gap-3 border-t border-white/[0.06] px-5 py-4">
      <span
        aria-hidden
        className="grid size-9 shrink-0 place-items-center rounded-full bg-[linear-gradient(135deg,#7a7df4,#4b4ed8)] text-sm font-semibold text-white"
      >
        {name?.[0]?.toUpperCase() ?? "?"}
      </span>
      <span className="min-w-0 flex-1 leading-tight">
        <span className="block truncate text-sm font-semibold">{name}</span>
        <span className="block text-xs text-faint">{role}</span>
      </span>
      <button
        type="button"
        onClick={async () => {
          await api.POST("/api/v1/auth/logout");
          qc.clear();
          navigate({ to: "/login" });
        }}
        className="grid size-8 place-items-center rounded-md text-faint hover:bg-raised hover:text-fg"
        title="Sign out"
      >
        <LogOut aria-hidden className="size-4" />
        <span className="sr-only">Sign out</span>
      </button>
    </div>
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
