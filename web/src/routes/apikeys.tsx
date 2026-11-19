import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Trash2 } from "lucide-react";
import { useState } from "react";
import { api, unwrap } from "../api/client";

/** Keys for other apps. The full key is shown once, right after it is made. */
export function ApiKeysSection() {
  const qc = useQueryClient();
  const { data: keys } = useQuery({ queryKey: ["api-keys"], queryFn: () => unwrap(api.GET("/api/v1/api-keys")) });
  const [name, setName] = useState("");
  const [fresh, setFresh] = useState<{ name: string; key: string } | null>(null);
  const invalidate = () => qc.invalidateQueries({ queryKey: ["api-keys"] });

  const add = useMutation({
    mutationFn: () => unwrap(api.POST("/api/v1/api-keys", { body: { name } })),
    onSuccess: (k) => {
      setFresh({ name: k.name, key: k.key });
      setName("");
      invalidate();
    },
  });
  const remove = useMutation({
    mutationFn: (id: number) => unwrap(api.DELETE("/api/v1/api-keys/{id}", { params: { path: { id } } })),
    onSuccess: invalidate,
  });

  return (
    <div className="mt-6">
      {fresh && (
        <div className="mb-6 border border-slate-line p-4">
          <p className="text-sm text-chalk-dim">Key for {fresh.name}. Copy it now: it will not be shown again.</p>
          <input className="chalk-input data mt-2 text-base" readOnly value={fresh.key} onFocus={(e) => e.target.select()} />
        </div>
      )}
      {keys && keys.length > 0 && (
        <ul className="mb-6 border-t border-slate-line">
          {keys.map((k) => (
            <li key={k.id} className="flex flex-wrap items-center gap-x-4 gap-y-1 border-b border-slate-line py-3">
              <span className="font-slate text-lg font-bold">{k.name}</span>
              <span className="flex-1 text-sm text-chalk-dim">
                {k.last_used_at ? `Last used ${new Date(k.last_used_at * 1000).toLocaleString()}` : "Never used"}
              </span>
              <button
                type="button"
                aria-label={`Delete key ${k.name}`}
                className="text-chalk-faint hover:text-grease-hot"
                onClick={() => remove.mutate(k.id)}
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
        className="flex flex-wrap items-end gap-x-5 gap-y-4"
      >
        <label className="block w-60">
          <span className="slate-label">Name</span>
          <input className="chalk-input" value={name} onChange={(e) => setName(e.target.value)} placeholder="Indexer manager" required />
        </label>
        <button type="submit" className="btn-grease" disabled={add.isPending}>
          Create key
        </button>
      </form>
      {(add.error ?? remove.error) && <p className="mt-3 text-sm text-grease-hot">{(add.error ?? remove.error)?.message}</p>}
    </div>
  );
}
