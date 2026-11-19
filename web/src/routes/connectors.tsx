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

/** qBittorrent connection. One client for now. */
export function DownloadClientSection() {
  const qc = useQueryClient();
  const { data: clients } = useQuery({
    queryKey: ["download-clients"],
    queryFn: () => unwrap(api.GET("/api/v1/download-clients")),
  });
  const current = clients?.[0];
  const [form, setForm] = useState({ base_url: "", username: "", password: "", category: "koda" });
  useEffect(() => {
    if (current)
      setForm({
        base_url: current.base_url,
        username: current.username ?? "",
        password: "",
        category: current.category,
      });
  }, [current]);

  const body = () => ({
    name: "qBittorrent",
    base_url: form.base_url,
    username: form.username || null,
    password: form.password || null,
    category: form.category,
  });
  const test = useMutation({ mutationFn: () => unwrap(api.POST("/api/v1/download-clients/test", { body: body() })) });
  const save = useMutation({
    mutationFn: () =>
      current
        ? unwrap(api.PUT("/api/v1/download-clients/{id}", { params: { path: { id: current.id } }, body: body() }))
        : unwrap(api.POST("/api/v1/download-clients", { body: body() })),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["download-clients"] }),
  });
  const set = (k: keyof typeof form) => (e: React.ChangeEvent<HTMLInputElement>) =>
    setForm((f) => ({ ...f, [k]: e.target.value }));

  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        save.mutate();
      }}
      className="mt-6 grid gap-6 sm:grid-cols-2"
    >
      <Field label="Web UI address" className="sm:col-span-2">
        <input
          className="chalk-input data text-base"
          value={form.base_url}
          onChange={set("base_url")}
          placeholder="http://192.168.1.10:8080"
          spellCheck={false}
          required
        />
      </Field>
      <Field label="Username">
        <input className="chalk-input" value={form.username} onChange={set("username")} autoComplete="off" />
      </Field>
      <Field label="Password">
        <input
          className="chalk-input"
          type="password"
          value={form.password}
          onChange={set("password")}
          autoComplete="new-password"
          placeholder={current?.password_set ? "Saved. Type to replace" : ""}
        />
      </Field>
      <Field label="Category">
        <input className="chalk-input data text-base" value={form.category} onChange={set("category")} />
      </Field>
      <div className="flex flex-wrap items-center gap-3 sm:col-span-2">
        <button type="submit" className="btn-grease" disabled={save.isPending}>
          {save.isPending ? "Saving…" : current ? "Save" : "Connect"}
        </button>
        <button type="button" className="btn-chalk" disabled={test.isPending} onClick={() => test.mutate()}>
          {test.isPending ? "Testing…" : "Test connection"}
        </button>
        <TestLine result={test.data} error={test.error} />
        {save.error && <p className="text-sm text-grease-hot">{save.error.message}</p>}
        {save.isSuccess && !save.error && <p className="text-sm text-tape-available">Saved.</p>}
      </div>
    </form>
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
