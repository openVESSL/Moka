import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Trash2 } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";
import { api, unwrap, type Schemas } from "../api/client";

type TestResult = Schemas["TestResult"];

function Field({ label, children, className = "" }: { label: string; children: ReactNode; className?: string }) {
  return (
    <label className={`block ${className}`}>
      <span className="slate-label">{label}</span>
      {children}
    </label>
  );
}

function TestLine({ result, error }: { result?: TestResult; error?: Error | null }) {
  if (error) return <p className="text-sm text-grease-hot">{error.message}</p>;
  if (!result) return null;
  return (
    <p role="status" className={`text-sm ${result.ok ? "text-tape-available" : "text-grease-hot"}`}>
      {result.message}
    </p>
  );
}

/** Library folder: where imported films are placed. */
export function LibrarySection() {
  const qc = useQueryClient();
  const { data } = useQuery({
    queryKey: ["settings", "library"],
    queryFn: () => unwrap(api.GET("/api/v1/settings/library")),
  });
  const [path, setPath] = useState("");
  useEffect(() => setPath(data?.movies_path ?? ""), [data]);
  const save = useMutation({
    mutationFn: () => unwrap(api.PUT("/api/v1/settings/library", { body: { movies_path: path } })),
    onSuccess: (v) => qc.setQueryData(["settings", "library"], v),
  });

  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        save.mutate();
      }}
      className="mt-6 flex flex-wrap items-end gap-x-5 gap-y-4"
    >
      <Field label="Films folder" className="min-w-0 flex-1 basis-80">
        <input
          className="chalk-input data text-base"
          value={path}
          onChange={(e) => setPath(e.target.value)}
          placeholder="/data/movies"
          spellCheck={false}
        />
      </Field>
      <button type="submit" className="btn-grease" disabled={save.isPending}>
        {save.isPending ? "Checking…" : "Save"}
      </button>
      <p role="status" className={`w-full text-sm ${save.error ? "text-grease-hot" : "text-tape-available"}`}>
        {save.error?.message ?? (save.isSuccess ? "Saved. The server can write there." : "")}
      </p>
    </form>
  );
}

const EMPTY_CLIENT = { kind: "qbittorrent", base_url: "", username: "", password: "", category: "koda" };
const KIND_LABEL: Record<string, string> = { qbittorrent: "qBittorrent", transmission: "Transmission", sabnzbd: "SABnzbd" };

/** Download clients: a torrent client, a Usenet client, or both. */
export function DownloadClientSection() {
  const qc = useQueryClient();
  const { data: clients } = useQuery({
    queryKey: ["download-clients"],
    queryFn: () => unwrap(api.GET("/api/v1/download-clients")),
  });
  const [form, setForm] = useState(EMPTY_CLIENT);
  const usenet = form.kind === "sabnzbd";
  const body = () => ({
    name: KIND_LABEL[form.kind],
    kind: form.kind,
    base_url: form.base_url,
    username: usenet ? null : form.username || null,
    password: form.password || null,
    category: form.category,
  });
  const invalidate = () => qc.invalidateQueries({ queryKey: ["download-clients"] });
  const test = useMutation({ mutationFn: () => unwrap(api.POST("/api/v1/download-clients/test", { body: body() })) });
  const add = useMutation({
    mutationFn: () => unwrap(api.POST("/api/v1/download-clients", { body: body() })),
    onSuccess: () => {
      setForm(EMPTY_CLIENT);
      test.reset();
      invalidate();
    },
  });
  const remove = useMutation({
    mutationFn: (id: number) => unwrap(api.DELETE("/api/v1/download-clients/{id}", { params: { path: { id } } })),
    onSuccess: invalidate,
  });
  const set = (k: keyof typeof form) => (e: React.ChangeEvent<HTMLInputElement | HTMLSelectElement>) =>
    setForm((f) => ({ ...f, [k]: e.target.value }));

  return (
    <div className="mt-6">
      {clients && clients.length > 0 && (
        <ul className="mb-8 border-t border-slate-line">
          {clients.map((c) => (
            <li key={c.id} className="flex flex-wrap items-center gap-x-4 gap-y-1 border-b border-slate-line py-3">
              <span className="font-slate text-lg font-bold">{c.name}</span>
              <span className="data min-w-0 flex-1 truncate text-sm text-chalk-dim">{c.base_url}</span>
              <span className="data text-xs text-chalk-faint">category {c.category}</span>
              <button
                type="button"
                className="text-chalk-faint hover:text-grease-hot"
                onClick={() => remove.mutate(c.id)}
                aria-label={`Remove ${c.name}`}
              >
                <Trash2 aria-hidden className="size-4" />
              </button>
            </li>
          ))}
        </ul>
      )}
      <form
        onSubmit={(e) => {
          e.preventDefault();
          add.mutate();
        }}
        className="grid gap-6 sm:grid-cols-[10rem_1fr]"
      >
        <Field label="Client">
          <select className="chalk-input bg-slate" value={form.kind} onChange={set("kind")}>
            <option value="qbittorrent">qBittorrent</option>
            <option value="transmission">Transmission</option>
            <option value="sabnzbd">SABnzbd (Usenet)</option>
          </select>
        </Field>
        <Field label="Web address">
          <input
            className="chalk-input data text-base"
            value={form.base_url}
            onChange={set("base_url")}
            placeholder={usenet ? "http://192.168.1.10:8080" : "http://192.168.1.10:8080"}
            spellCheck={false}
            required
          />
        </Field>
        {!usenet && (
          <Field label="Username">
            <input className="chalk-input" value={form.username} onChange={set("username")} autoComplete="off" />
          </Field>
        )}
        <Field label={usenet ? "API key" : "Password"}>
          <input
            className="chalk-input"
            type="password"
            value={form.password}
            onChange={set("password")}
            autoComplete="new-password"
          />
        </Field>
        <Field label="Category">
          <input className="chalk-input data text-base" value={form.category} onChange={set("category")} />
        </Field>
        <div className="flex flex-wrap items-center gap-3 sm:col-span-2">
          <button type="submit" className="btn-grease" disabled={add.isPending}>
            {add.isPending ? "Adding…" : "Add client"}
          </button>
          <button type="button" className="btn-chalk" disabled={test.isPending} onClick={() => test.mutate()}>
            {test.isPending ? "Testing…" : "Test connection"}
          </button>
          <TestLine result={test.data} error={test.error} />
          {(add.error ?? remove.error) && <p className="text-sm text-grease-hot">{(add.error ?? remove.error)?.message}</p>}
        </div>
      </form>
    </div>
  );
}

const EMPTY_INDEXER = { name: "", kind: "torznab", base_url: "", api_path: "/api", api_key: "", priority: "25" };

/** Torznab / Newznab indexers: list, add, remove. */
export function IndexersSection() {
  const qc = useQueryClient();
  const { data: indexers } = useQuery({
    queryKey: ["indexers"],
    queryFn: () => unwrap(api.GET("/api/v1/indexers")),
  });
  const [form, setForm] = useState(EMPTY_INDEXER);
  const body = () => ({
    name: form.name,
    kind: form.kind,
    base_url: form.base_url,
    api_path: form.api_path,
    api_key: form.api_key || null,
    priority: Number(form.priority) || 25,
  });
  const test = useMutation({ mutationFn: () => unwrap(api.POST("/api/v1/indexers/test", { body: body() })) });
  const add = useMutation({
    mutationFn: () => unwrap(api.POST("/api/v1/indexers", { body: body() })),
    onSuccess: () => {
      setForm(EMPTY_INDEXER);
      test.reset();
      qc.invalidateQueries({ queryKey: ["indexers"] });
    },
  });
  const remove = useMutation({
    mutationFn: (id: number) => unwrap(api.DELETE("/api/v1/indexers/{id}", { params: { path: { id } } })),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["indexers"] }),
  });
  const set = (k: keyof typeof form) => (e: React.ChangeEvent<HTMLInputElement | HTMLSelectElement>) =>
    setForm((f) => ({ ...f, [k]: e.target.value }));
  const paused = (until?: number | null) => until && until > Date.now() / 1000;

  return (
    <div className="mt-6">
      <ProwlarrImport />
      {indexers && indexers.length > 0 && (
        <ul className="mb-8 border-t border-slate-line">
          {indexers.map((i) => (
            <li key={i.id} className="flex flex-wrap items-center gap-x-4 gap-y-1 border-b border-slate-line py-3">
              <span className="font-slate text-lg font-bold">{i.name}</span>
              <span className="data text-xs text-chalk-faint uppercase">{i.kind}</span>
              <span className="data min-w-0 flex-1 truncate text-sm text-chalk-dim">{i.base_url}</span>
              {paused(i.paused_until) && (
                <span className="text-sm text-grease-hot" title={i.last_error ?? undefined}>
                  Paused: {i.last_error}
                </span>
              )}
              <button
                type="button"
                className="text-chalk-faint hover:text-grease-hot"
                onClick={() => remove.mutate(i.id)}
                aria-label={`Remove ${i.name}`}
              >
                <Trash2 aria-hidden className="size-4" />
              </button>
            </li>
          ))}
        </ul>
      )}
      <form
        onSubmit={(e) => {
          e.preventDefault();
          add.mutate();
        }}
        className="grid gap-6 sm:grid-cols-[1fr_8rem]"
      >
        <Field label="Name">
          <input className="chalk-input" value={form.name} onChange={set("name")} placeholder="My tracker" required />
        </Field>
        <Field label="Type">
          <select className="chalk-input bg-slate" value={form.kind} onChange={set("kind")}>
            <option value="torznab">Torznab</option>
            <option value="newznab">Newznab</option>
          </select>
        </Field>
        <Field label="Address" className="sm:col-span-2">
          <input
            className="chalk-input data text-base"
            value={form.base_url}
            onChange={set("base_url")}
            placeholder="http://localhost:9117/api/v2.0/indexers/example/results/torznab"
            spellCheck={false}
            required
          />
        </Field>
        <Field label="API key">
          <input
            className="chalk-input data text-base"
            type="password"
            value={form.api_key}
            onChange={set("api_key")}
            autoComplete="off"
          />
        </Field>
        <Field label="API path">
          <input className="chalk-input data text-base" value={form.api_path} onChange={set("api_path")} />
        </Field>
        <div className="flex flex-wrap items-center gap-3 sm:col-span-2">
          <button type="submit" className="btn-grease" disabled={add.isPending}>
            {add.isPending ? "Adding…" : "Add indexer"}
          </button>
          <button type="button" className="btn-chalk" disabled={test.isPending} onClick={() => test.mutate()}>
            {test.isPending ? "Testing…" : "Test"}
          </button>
          <TestLine result={test.data} error={test.error} />
          {add.error && <p className="text-sm text-grease-hot">{add.error.message}</p>}
        </div>
      </form>
    </div>
  );
}

/** Adds films already in the films folder. Shows progress while it runs. */
export function ScanPanel() {
  const qc = useQueryClient();
  const { data: s } = useQuery({
    queryKey: ["library", "scan"],
    queryFn: () => unwrap(api.GET("/api/v1/library/scan")),
    refetchInterval: (q) => (q.state.data?.running ? 1500 : false),
  });
  const start = useMutation({
    mutationFn: () => unwrap(api.POST("/api/v1/library/scan", { body: {} })),
    onSuccess: (v) => qc.setQueryData(["library", "scan"], v),
  });
  useEffect(() => {
    if (s && !s.running && s.done > 0) qc.invalidateQueries({ queryKey: ["titles"] });
  }, [s, qc]);

  return (
    <div className="mt-6 border-t border-slate-line pt-5">
      <div className="flex flex-wrap items-center gap-4">
        <button type="button" className="btn-chalk" disabled={start.isPending || s?.running} onClick={() => start.mutate()}>
          {s?.running ? "Scanning…" : "Find films already in this folder"}
        </button>
        {s && (s.running || s.done > 0) && (
          <span className="data text-sm text-chalk-dim">
            {s.done}/{s.total} checked · {s.added} added · {s.already_known} known · {s.unmatched.length} not recognized
          </span>
        )}
      </div>
      {start.error && <p className="mt-2 text-sm text-grease-hot">{start.error.message}</p>}
      {s?.error && <p className="mt-2 text-sm text-grease-hot">{s.error}</p>}
      {s && !s.running && s.unmatched.length > 0 && (
        <details className="mt-3 text-sm text-chalk-dim">
          <summary className="cursor-pointer">Not recognized ({s.unmatched.length})</summary>
          <p className="mt-2 max-w-[65ch]">
            Rename these folders to <span className="data">Title (Year)</span>, or add{" "}
            <span className="data">{"{tmdb-ID}"}</span>, then scan again.
          </p>
          <ul className="mt-2 data text-xs">
            {s.unmatched.map((n) => (
              <li key={n}>{n}</li>
            ))}
          </ul>
        </details>
      )}
    </div>
  );
}

/** Optional Jellyfin connection: refresh after import, watched status back. */
export function JellyfinSection() {
  const qc = useQueryClient();
  const { data } = useQuery({
    queryKey: ["settings", "jellyfin"],
    queryFn: () => unwrap(api.GET("/api/v1/settings/jellyfin")),
  });
  const [f, setF] = useState({ enabled: false, url: "", api_key: "", path_from: "", path_to: "", sync_watched: true });
  useEffect(() => {
    if (data) setF({ ...data, api_key: "" });
  }, [data]);
  const save = useMutation({
    mutationFn: () =>
      unwrap(api.PUT("/api/v1/settings/jellyfin", { body: { ...f, api_key: f.api_key || null } })),
    onSuccess: (v) => {
      setF((x) => ({ ...x, api_key: "" }));
      qc.setQueryData(["settings", "jellyfin"], v);
    },
  });
  const text = (k: "url" | "api_key" | "path_from" | "path_to") => (e: React.ChangeEvent<HTMLInputElement>) =>
    setF((x) => ({ ...x, [k]: e.target.value }));

  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        save.mutate();
      }}
      className="mt-6 grid gap-6 sm:grid-cols-2"
    >
      <label className="inline-flex items-center gap-2 text-sm text-chalk-dim sm:col-span-2">
        <input
          type="checkbox"
          className="accent-grease"
          checked={f.enabled}
          onChange={(e) => setF((x) => ({ ...x, enabled: e.target.checked }))}
        />
        Use Jellyfin alongside Koda
      </label>
      <Field label="Jellyfin address">
        <input className="chalk-input data text-base" value={f.url} onChange={text("url")} placeholder="http://192.168.1.10:8096" />
      </Field>
      <Field label="API key">
        <input
          className="chalk-input data text-base"
          type="password"
          value={f.api_key}
          onChange={text("api_key")}
          placeholder={data?.api_key_set ? "Saved. Paste a new one to replace it" : "Dashboard → API Keys"}
          autoComplete="off"
        />
      </Field>
      <Field label="Library path in Koda">
        <input className="chalk-input data text-base" value={f.path_from} onChange={text("path_from")} placeholder="/data/movies (only if different)" />
      </Field>
      <Field label="Same folder in Jellyfin">
        <input className="chalk-input data text-base" value={f.path_to} onChange={text("path_to")} placeholder="/media/movies" />
      </Field>
      <label className="inline-flex items-center gap-2 text-sm text-chalk-dim sm:col-span-2">
        <input
          type="checkbox"
          className="accent-grease"
          checked={f.sync_watched}
          onChange={(e) => setF((x) => ({ ...x, sync_watched: e.target.checked }))}
        />
        Mark films watched in Jellyfin as watched here (people are matched by user name)
      </label>
      <div className="flex flex-wrap items-center gap-3 sm:col-span-2">
        <button type="submit" className="btn-grease" disabled={save.isPending}>
          {save.isPending ? "Checking…" : "Save"}
        </button>
        <p role="status" className={`text-sm ${save.error ? "text-grease-hot" : "text-tape-available"}`}>
          {save.error?.message ?? (save.isSuccess ? "Saved. New films will show up in Jellyfin right after import." : "")}
        </p>
      </div>
    </form>
  );
}

/** Radarr in front: approved films go to Radarr, its films show up here. */
export function RadarrSection() {
  const qc = useQueryClient();
  const { data } = useQuery({
    queryKey: ["settings", "radarr"],
    queryFn: () => unwrap(api.GET("/api/v1/settings/radarr")),
  });
  const [f, setF] = useState({
    enabled: false,
    url: "",
    api_key: "",
    quality_profile_id: 0,
    root_folder: "",
    path_from: "",
    path_to: "",
  });
  useEffect(() => {
    if (data) setF({ ...data, api_key: "" });
  }, [data]);
  const save = useMutation({
    mutationFn: () => unwrap(api.PUT("/api/v1/settings/radarr", { body: { ...f, api_key: f.api_key || null } })),
    onSuccess: (v) => {
      setF((x) => ({ ...x, api_key: "" }));
      qc.setQueryData(["settings", "radarr"], v);
    },
  });
  const text = (k: "url" | "api_key" | "root_folder" | "path_from" | "path_to") =>
    (e: React.ChangeEvent<HTMLInputElement | HTMLSelectElement>) => setF((x) => ({ ...x, [k]: e.target.value }));

  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        save.mutate();
      }}
      className="mt-6 grid gap-6 sm:grid-cols-2"
    >
      <label className="inline-flex items-center gap-2 text-sm text-chalk-dim sm:col-span-2">
        <input
          type="checkbox"
          className="accent-grease"
          checked={f.enabled}
          onChange={(e) => setF((x) => ({ ...x, enabled: e.target.checked }))}
        />
        Let Radarr search and download (Koda's own search stops)
      </label>
      <Field label="Radarr address">
        <input className="chalk-input data text-base" value={f.url} onChange={text("url")} placeholder="http://192.168.1.10:7878" />
      </Field>
      <Field label="API key">
        <input
          className="chalk-input data text-base"
          type="password"
          value={f.api_key}
          onChange={text("api_key")}
          placeholder={data?.api_key_set ? "Saved. Paste a new one to replace it" : "Settings → General → API Key"}
          autoComplete="off"
        />
      </Field>
      {data && data.profiles.length > 0 && (
        <Field label="Quality profile">
          <select
            className="chalk-input bg-slate"
            value={f.quality_profile_id}
            onChange={(e) => setF((x) => ({ ...x, quality_profile_id: Number(e.target.value) }))}
          >
            <option value={0}>First one ({data.profiles[0].name})</option>
            {data.profiles.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name}
              </option>
            ))}
          </select>
        </Field>
      )}
      {data && data.root_folders.length > 0 && (
        <Field label="Root folder">
          <select className="chalk-input bg-slate" value={f.root_folder} onChange={text("root_folder")}>
            <option value="">First one ({data.root_folders[0]})</option>
            {data.root_folders.map((r) => (
              <option key={r} value={r}>
                {r}
              </option>
            ))}
          </select>
        </Field>
      )}
      <Field label="Film folder in Radarr">
        <input className="chalk-input data text-base" value={f.path_from} onChange={text("path_from")} placeholder="/movies (only if different)" />
      </Field>
      <Field label="Same folder in Koda">
        <input className="chalk-input data text-base" value={f.path_to} onChange={text("path_to")} placeholder="/data/movies" />
      </Field>
      <div className="flex flex-wrap items-center gap-3 sm:col-span-2">
        <button type="submit" className="btn-grease" disabled={save.isPending}>
          {save.isPending ? "Checking…" : "Save"}
        </button>
        <p role="status" className={`text-sm ${save.error || data?.error ? "text-grease-hot" : "text-tape-available"}`}>
          {save.error?.message ??
            data?.error ??
            (save.isSuccess ? "Saved. Radarr's films are being added to the library." : "")}
        </p>
      </div>
    </form>
  );
}

/** Brings in every film indexer already set up in Prowlarr. */
export function ProwlarrImport() {
  const qc = useQueryClient();
  const [f, setF] = useState({ url: "", api_key: "" });
  const run = useMutation({
    mutationFn: () => unwrap(api.POST("/api/v1/indexers/import/prowlarr", { body: f })),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["indexers"] }),
  });
  const r = run.data;
  return (
    <details className="mb-8 border-y border-slate-line py-4">
      <summary className="cursor-pointer font-slate font-bold tracking-[0.06em] uppercase">Import from Prowlarr</summary>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          run.mutate();
        }}
        className="mt-4 flex flex-wrap items-end gap-x-5 gap-y-4"
      >
        <Field label="Prowlarr address" className="min-w-0 flex-1 basis-64">
          <input
            className="chalk-input data text-base"
            value={f.url}
            onChange={(e) => setF((x) => ({ ...x, url: e.target.value }))}
            placeholder="http://192.168.1.10:9696"
            required
          />
        </Field>
        <Field label="API key" className="w-64">
          <input
            className="chalk-input data text-base"
            type="password"
            value={f.api_key}
            onChange={(e) => setF((x) => ({ ...x, api_key: e.target.value }))}
            autoComplete="off"
            required
          />
        </Field>
        <button type="submit" className="btn-grease" disabled={run.isPending}>
          {run.isPending ? "Importing…" : "Import"}
        </button>
      </form>
      {run.error && <p className="mt-3 text-sm text-grease-hot">{run.error.message}</p>}
      {r && (
        <p className="mt-3 text-sm text-chalk-dim">
          {r.added.length} added, {r.updated.length} updated
          {r.skipped.length > 0 && <> · skipped: {r.skipped.join("; ")}</>}
        </p>
      )}
      <p className="mt-2 text-xs text-chalk-faint">
        Searches go through Prowlarr, so it must stay reachable. Run it again after changing indexers there.
      </p>
    </details>
  );
}
