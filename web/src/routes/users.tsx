import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Trash2 } from "lucide-react";
import { useEffect, useState } from "react";
import { api, unwrap, type Schemas } from "../api/client";

const PERMS = [
  { bit: 1, label: "Admin" },
  { bit: 2, label: "Manage requests" },
  { bit: 4, label: "Request" },
  { bit: 8, label: "Auto-approve" },
  { bit: 16, label: "Manage library" },
  { bit: 32, label: "Play" },
  { bit: 128, label: "Request films" },
  { bit: 256, label: "Request shows" },
  { bit: 512, label: "Auto-approve films" },
  { bit: 1024, label: "Auto-approve shows" },
] as const;

type User = Schemas["UserView"];
type UserUpdate = Schemas["UserUpdate"];

/** "3 every 7 days" fields; an empty number means the global limit. */
function LimitField({ label, value, onChange }: { label: string; value: { limit: string; days: string }; onChange: (v: { limit: string; days: string }) => void }) {
  return (
    <fieldset className="flex items-end gap-2">
      <legend className="field-label">{label}</legend>
      <input
        className="field data w-20 text-sm"
        inputMode="numeric"
        value={value.limit}
        onChange={(e) => onChange({ ...value, limit: e.target.value.replace(/\D/g, "") })}
        placeholder="global"
        aria-label={`${label}: requests`}
      />
      <span className="pb-2 text-sm text-muted">every</span>
      <input
        className="field data w-16 text-sm"
        inputMode="numeric"
        value={value.days}
        onChange={(e) => onChange({ ...value, days: e.target.value.replace(/\D/g, "") })}
        aria-label={`${label}: days`}
      />
      <span className="pb-2 text-sm text-muted">days</span>
    </fieldset>
  );
}

/** Per person, like a request app: own limits for films and shows, and the bot's messages. */
function RequestLimits({ user, onSave }: { user: User; onSave: (id: number, body: UserUpdate) => void }) {
  const toField = (q?: { limit: number; days: number } | null) => ({ limit: q ? String(q.limit) : "", days: String(q?.days ?? 7) });
  const [films, setFilms] = useState(toField(user.movie_quota));
  const [shows, setShows] = useState(toField(user.show_quota));
  const [notify, setNotify] = useState(user.discord_notify);
  useEffect(() => {
    setFilms(toField(user.movie_quota));
    setShows(toField(user.show_quota));
    setNotify(user.discord_notify);
  }, [user]);
  const quota = (f: { limit: string; days: string }) => ({ limit: Number(f.limit), days: Number(f.days) || 7 });
  return (
    <form
      className="flex w-full flex-wrap items-end gap-x-6 gap-y-3 pb-1"
      onSubmit={(e) => {
        e.preventDefault();
        onSave(user.id, {
          discord_notify: notify,
          ...(films.limit === "" ? { clear_movie_quota: true } : { movie_quota: quota(films) }),
          ...(shows.limit === "" ? { clear_show_quota: true } : { show_quota: quota(shows) }),
        });
      }}
    >
      <LimitField label="Film limit" value={films} onChange={setFilms} />
      <LimitField label="Show limit" value={shows} onChange={setShows} />
      <label className="inline-flex items-center gap-1.5 pb-2 text-sm text-muted">
        <input type="checkbox" className="accent-accent" checked={notify} onChange={(e) => setNotify(e.target.checked)} />
        Discord message when ready
      </label>
      <button type="submit" className="btn-primary">
        Save
      </button>
    </form>
  );
}

/** The person's Discord account, so the Discord bot requests in their name. Saved when the field is left. */
function DiscordId({ id, value, onSave }: { id: number; value: string; onSave: (id: number, discord_id: string) => void }) {
  const [v, setV] = useState(value);
  useEffect(() => setV(value), [value]);
  return (
    <input
      className="field data w-48 text-sm"
      value={v}
      onChange={(e) => setV(e.target.value.trim())}
      onBlur={() => v !== value && onSave(id, v)}
      placeholder="Discord ID"
      aria-label="Discord ID"
      inputMode="numeric"
      title="In Discord: Settings → Advanced → Developer Mode, then right-click the person → Copy User ID"
    />
  );
}

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
  const [open, setOpen] = useState<number | null>(null);
  const setLimits = useMutation({
    mutationFn: ({ id, body }: { id: number; body: UserUpdate }) =>
      unwrap(api.PUT("/api/v1/users/{id}", { params: { path: { id } }, body })),
    onSettled: invalidate,
  });
  const setDiscord = useMutation({
    mutationFn: ({ id, discord_id }: { id: number; discord_id: string }) =>
      unwrap(api.PUT("/api/v1/users/{id}", { params: { path: { id } }, body: { discord_id } })),
    onSettled: invalidate,
  });
  const remove = useMutation({
    mutationFn: (id: number) => unwrap(api.DELETE("/api/v1/users/{id}", { params: { path: { id } } })),
    onSuccess: invalidate,
  });
  const error = create.error ?? setPerms.error ?? setDiscord.error ?? setLimits.error ?? remove.error;

  return (
    <div className="mt-6">
      <ul className="border-t border-line">
        {users?.map((u) => (
          <li key={u.id} className="flex flex-wrap items-center gap-x-5 gap-y-2 border-b border-line py-3">
            <span className="w-40 text-lg font-bold">{u.username}</span>
            <span className="flex flex-1 flex-wrap gap-x-4 gap-y-1">
              {PERMS.map((p) => (
                <label key={p.bit} className="inline-flex items-center gap-1.5 text-sm text-muted">
                  <input
                    type="checkbox"
                    className="accent-accent"
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
            <DiscordId id={u.id} value={u.discord_id ?? ""} onSave={(id, discord_id) => setDiscord.mutate({ id, discord_id })} />
            <button
              type="button"
              className="text-sm text-muted underline-offset-4 hover:underline"
              aria-expanded={open === u.id}
              onClick={() => setOpen(open === u.id ? null : u.id)}
            >
              Limits
            </button>
            <button
              type="button"
              aria-label={`Delete ${u.username}`}
              className="text-faint hover:text-danger"
              onClick={() => remove.mutate(u.id)}
            >
              <Trash2 aria-hidden className="size-4" />
            </button>
            {open === u.id && <RequestLimits user={u} onSave={(id, body) => setLimits.mutate({ id, body })} />}
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
          <span className="field-label">Name</span>
          <input
            className="field"
            value={form.username}
            onChange={(e) => setForm((f) => ({ ...f, username: e.target.value }))}
            autoComplete="off"
            required
          />
        </label>
        <label className="block w-56">
          <span className="field-label">Password</span>
          <input
            className="field"
            type="password"
            value={form.password}
            onChange={(e) => setForm((f) => ({ ...f, password: e.target.value }))}
            autoComplete="new-password"
            minLength={8}
            required
          />
        </label>
        <button type="submit" className="btn-primary" disabled={create.isPending}>
          Add person
        </button>
      </form>
      {error && <p className="mt-3 text-sm text-danger">{error.message}</p>}
    </div>
  );
}
