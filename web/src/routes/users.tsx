import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Trash2 } from "lucide-react";
import { useState } from "react";
import { api, unwrap } from "../api/client";

const PERMS = [
  { bit: 1, label: "Admin" },
  { bit: 2, label: "Manage requests" },
  { bit: 4, label: "Request" },
  { bit: 8, label: "Auto-approve" },
  { bit: 16, label: "Manage library" },
  { bit: 32, label: "Play" },
] as const;

/** Accounts: create, set permissions, remove. */
export function UsersSection() {
  const qc = useQueryClient();
  const { data: users } = useQuery({ queryKey: ["users"], queryFn: () => unwrap(api.GET("/api/v1/users")) });
  const [form, setForm] = useState({ username: "", password: "" });
  const invalidate = () => qc.invalidateQueries({ queryKey: ["users"] });

  const create = useMutation({
    mutationFn: () => unwrap(api.POST("/api/v1/users", { body: form })),
    onSuccess: () => {
      setForm({ username: "", password: "" });
      invalidate();
    },
  });
  const setPerms = useMutation({
    mutationFn: ({ id, permissions }: { id: number; permissions: number }) =>
      unwrap(api.PUT("/api/v1/users/{id}", { params: { path: { id } }, body: { permissions } })),
    onSuccess: invalidate,
  });
  const remove = useMutation({
    mutationFn: (id: number) => unwrap(api.DELETE("/api/v1/users/{id}", { params: { path: { id } } })),
    onSuccess: invalidate,
  });
  const error = create.error ?? setPerms.error ?? remove.error;

  return (
    <div className="mt-6">
      <ul className="border-t border-slate-line">
        {users?.map((u) => (
          <li key={u.id} className="flex flex-wrap items-center gap-x-5 gap-y-2 border-b border-slate-line py-3">
            <span className="w-40 font-slate text-lg font-bold">{u.username}</span>
            <span className="flex flex-1 flex-wrap gap-x-4 gap-y-1">
              {PERMS.map((p) => (
                <label key={p.bit} className="inline-flex items-center gap-1.5 text-sm text-chalk-dim">
                  <input
                    type="checkbox"
                    className="accent-grease"
                    checked={(u.permissions & p.bit) !== 0}
                    onChange={(e) =>
                      setPerms.mutate({
                        id: u.id,
                        permissions: e.target.checked ? u.permissions | p.bit : u.permissions & ~p.bit,
                      })
                    }
                  />
                  {p.label}
                </label>
              ))}
            </span>
            <button
              type="button"
              aria-label={`Delete ${u.username}`}
              className="text-chalk-faint hover:text-grease-hot"
              onClick={() => remove.mutate(u.id)}
            >
              <Trash2 aria-hidden className="size-4" />
            </button>
          </li>
        ))}
      </ul>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          create.mutate();
        }}
        className="mt-6 flex flex-wrap items-end gap-x-5 gap-y-4"
      >
        <label className="block w-48">
          <span className="slate-label">Name</span>
          <input
            className="chalk-input"
            value={form.username}
            onChange={(e) => setForm((f) => ({ ...f, username: e.target.value }))}
            autoComplete="off"
            required
          />
        </label>
        <label className="block w-56">
          <span className="slate-label">Password</span>
          <input
            className="chalk-input"
            type="password"
            value={form.password}
            onChange={(e) => setForm((f) => ({ ...f, password: e.target.value }))}
            autoComplete="new-password"
            minLength={8}
            required
          />
        </label>
        <button type="submit" className="btn-grease" disabled={create.isPending}>
          Add person
        </button>
      </form>
      {error && <p className="mt-3 text-sm text-grease-hot">{error.message}</p>}
    </div>
  );
}
