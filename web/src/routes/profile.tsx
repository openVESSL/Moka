import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ArrowDown, ArrowUp } from "lucide-react";
import { useEffect, useState } from "react";
import { api, unwrap, type Schemas } from "../api/client";

type Profile = Schemas["QualityProfile"];

/** Which qualities to accept, best first, and when to stop upgrading. */
export function ProfileSection() {
  const qc = useQueryClient();
  const { data: options } = useQuery({ queryKey: ["qualities"], queryFn: () => unwrap(api.GET("/api/v1/qualities")) });
  const { data: saved } = useQuery({
    queryKey: ["quality-profile"],
    queryFn: () => unwrap(api.GET("/api/v1/quality-profile")),
  });
  const [p, setP] = useState<Profile>();
  useEffect(() => setP(saved), [saved]);
  const save = useMutation({
    mutationFn: (body: Profile) => unwrap(api.PUT("/api/v1/quality-profile", { body })),
    onSuccess: (v) => qc.setQueryData(["quality-profile"], v),
  });
  if (!p || !options) return null;

  const label = (k: string) => options.find((o) => o.key === k)?.label ?? k;
  const unused = options.filter((o) => !p.qualities.includes(o.key)).reverse();
  const move = (i: number, d: -1 | 1) => {
    const q = [...p.qualities];
    [q[i], q[i + d]] = [q[i + d], q[i]];
    setP({ ...p, qualities: q });
  };

  return (
    <div className="mt-6">
      <ol className="border-t border-line">
        {p.qualities.map((k, i) => (
          <li key={k} className="flex items-center gap-3 border-b border-line py-2">
            <span className="data w-6 text-xs text-faint">{i + 1}</span>
            <span className="flex-1 font-semibold">{label(k)}</span>
            <label className="inline-flex items-center gap-1.5 text-xs text-muted">
              <input
                type="radio"
                name="cutoff"
                className="accent-accent"
                checked={p.cutoff === k}
                onChange={() => setP({ ...p, cutoff: k })}
              />
              stop upgrading here
            </label>
            <button type="button" aria-label="Move up" disabled={i === 0} onClick={() => move(i, -1)} className="text-muted disabled:opacity-30">
              <ArrowUp aria-hidden className="size-4" />
            </button>
            <button type="button" aria-label="Move down" disabled={i === p.qualities.length - 1} onClick={() => move(i, 1)} className="text-muted disabled:opacity-30">
              <ArrowDown aria-hidden className="size-4" />
            </button>
            <button
              type="button"
              className="text-xs text-faint hover:text-danger"
              disabled={p.qualities.length === 1}
              onClick={() =>
                setP({
                  ...p,
                  qualities: p.qualities.filter((x) => x !== k),
                  cutoff: p.cutoff === k ? p.qualities.find((x) => x !== k)! : p.cutoff,
                })
              }
            >
              Remove
            </button>
          </li>
        ))}
      </ol>
      <div className="mt-5 flex flex-wrap items-end gap-x-5 gap-y-4">
        <label className="block">
          <span className="field-label">Add a quality</span>
          <select
            className="field bg-canvas text-base"
            value=""
            onChange={(e) => e.target.value && setP({ ...p, qualities: [...p.qualities, e.target.value] })}
          >
            <option value="">Choose…</option>
            {unused.map((o) => (
              <option key={o.key} value={o.key}>
                {o.label}
              </option>
            ))}
          </select>
        </label>
        <label className="block w-32">
          <span className="field-label">Min. seeders</span>
          <input
            className="field data"
            inputMode="numeric"
            value={p.min_seeders}
            onChange={(e) => setP({ ...p, min_seeders: Number(e.target.value.replace(/\D/g, "")) || 0 })}
          />
        </label>
        <label className="inline-flex items-center gap-2 pb-3 text-sm text-muted">
          <input
            type="checkbox"
            className="accent-accent"
            checked={p.upgrades_allowed}
            onChange={(e) => setP({ ...p, upgrades_allowed: e.target.checked })}
          />
          Replace with better releases when they appear
        </label>
        <button type="button" className="btn-primary" disabled={save.isPending} onClick={() => save.mutate(p)}>
          Save
        </button>
      </div>
      <p role="status" className={`mt-3 text-sm ${save.error ? "text-danger" : "text-state-available"}`}>
        {save.error?.message ?? (save.isSuccess ? "Saved." : "")}
      </p>
    </div>
  );
}

/** Audio formats music may be downloaded in. The server ranks them best first. */
export function MusicProfileSection() {
  const qc = useQueryClient();
  const { data: saved } = useQuery({
    queryKey: ["settings", "music"],
    queryFn: () => unwrap(api.GET("/api/v1/settings/music")),
  });
  const [picked, setPicked] = useState<string[]>([]);
  const [seeders, setSeeders] = useState(1);
  useEffect(() => {
    setPicked(saved?.qualities ?? []);
    setSeeders(saved?.min_seeders ?? 1);
  }, [saved]);
  const save = useMutation({
    mutationFn: () =>
      unwrap(api.PUT("/api/v1/settings/music", { body: { qualities: picked, min_seeders: seeders } })),
    onSuccess: (v) => qc.setQueryData(["settings", "music"], v),
  });
  if (!saved) return null;

  return (
    <div className="mt-10">
      <h3 className="text-base font-semibold">Music</h3>
      <p className="mt-1 max-w-2xl text-sm text-muted">
        Formats an album may be downloaded in, best first. The best one found wins.
      </p>
      <ul className="mt-4 grid max-w-2xl gap-x-6 sm:grid-cols-2">
        {saved.formats.map((f) => (
          <li key={f.key}>
            <label className="flex items-center gap-2.5 border-b border-line py-2.5">
              <input
                type="checkbox"
                className="accent-accent"
                checked={picked.includes(f.key)}
                onChange={(e) =>
                  setPicked(e.target.checked ? [...picked, f.key] : picked.filter((k) => k !== f.key))
                }
              />
              <span className="font-semibold">{f.label}</span>
            </label>
          </li>
        ))}
      </ul>
      <div className="mt-5 flex flex-wrap items-end gap-x-5 gap-y-4">
        <label className="block w-32">
          <span className="field-label">Min. seeders</span>
          <input
            className="field data"
            inputMode="numeric"
            value={seeders}
            onChange={(e) => setSeeders(Number(e.target.value.replace(/\D/g, "")) || 0)}
          />
        </label>
        <button
          type="button"
          className="btn-primary"
          disabled={save.isPending || picked.length === 0}
          onClick={() => save.mutate()}
        >
          Save
        </button>
      </div>
      <p role="status" className={`mt-3 text-sm ${save.error ? "text-danger" : "text-state-available"}`}>
        {save.error?.message ?? (picked.length === 0 ? "Pick at least one format." : save.isSuccess ? "Saved." : "")}
      </p>
    </div>
  );
}
