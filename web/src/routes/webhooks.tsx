import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Trash2 } from "lucide-react";
import { useState } from "react";
import { api, unwrap } from "../api/client";

/** Webhooks for bots and chat: a Discord webhook address gets a formatted message. */
export function WebhooksSection() {
  const qc = useQueryClient();
  const { data: hooks } = useQuery({ queryKey: ["webhooks"], queryFn: () => unwrap(api.GET("/api/v1/webhooks")) });
  const [form, setForm] = useState({ name: "", url: "" });
  const [tested, setTested] = useState<Record<number, string>>({});
  const invalidate = () => qc.invalidateQueries({ queryKey: ["webhooks"] });

  const add = useMutation({
    mutationFn: () => unwrap(api.POST("/api/v1/webhooks", { body: form })),
    onSuccess: () => {
      setForm({ name: "", url: "" });
      invalidate();
    },
  });
  const remove = useMutation({
    mutationFn: (id: number) => unwrap(api.DELETE("/api/v1/webhooks/{id}", { params: { path: { id } } })),
    onSuccess: invalidate,
  });
  const test = useMutation({
    mutationFn: (id: number) => unwrap(api.POST("/api/v1/webhooks/{id}/test", { params: { path: { id } } })),
    onSuccess: (r, id) => setTested((t) => ({ ...t, [id]: r.message })),
  });

  return (
    <div className="mt-6">
      {hooks && hooks.length > 0 && (
        <ul className="mb-6 border-t border-line">
          {hooks.map((h) => (
            <li key={h.id} className="flex flex-wrap items-center gap-x-4 gap-y-1 border-b border-line py-3">
              <span className="text-lg font-bold">{h.name}</span>
              <span className="data min-w-0 flex-1 truncate text-sm text-muted">{h.url}</span>
              {tested[h.id] && <span className="text-sm text-muted">{tested[h.id]}</span>}
              <button type="button" className="btn min-h-8 px-3 text-xs" onClick={() => test.mutate(h.id)}>
                Send test
              </button>
              <button
                type="button"
                aria-label={`Remove ${h.name}`}
                className="text-faint hover:text-danger"
                onClick={() => remove.mutate(h.id)}
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
        <label className="block w-40">
          <span className="field-label">Name</span>
          <input
            className="field"
            value={form.name}
            onChange={(e) => setForm((f) => ({ ...f, name: e.target.value }))}
            placeholder="Discord"
          />
        </label>
        <label className="block min-w-0 flex-1 basis-72">
          <span className="field-label">Address</span>
          <input
            className="field data text-base"
            value={form.url}
            onChange={(e) => setForm((f) => ({ ...f, url: e.target.value }))}
            placeholder="https://discord.com/api/webhooks/..."
            required
          />
        </label>
        <button type="submit" className="btn-primary" disabled={add.isPending}>
          Add webhook
        </button>
      </form>
      {(add.error ?? test.error) && <p className="mt-3 text-sm text-danger">{(add.error ?? test.error)?.message}</p>}
    </div>
  );
}
