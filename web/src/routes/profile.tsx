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
