import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useState, type ReactNode } from "react";
import { api, unwrap, type Schemas } from "../api/client";

type View = Schemas["AnchorrView"];

function Field({ label, hint, children }: { label: string; hint?: ReactNode; children: ReactNode }) {
  return (
    <label className="block">
      <span className="field-label">{label}</span>
      {children}
      {hint && <span className="mt-1 block text-xs text-faint">{hint}</span>}
    </label>
  );
}

/** What the bot is doing right now, with the next step when one is missing. */
function BotState({ view }: { view: View }) {
  const bot = view.bot;
  if (!view.enabled) return null;
  if (view.error) return <p className="text-sm text-danger">{view.error}</p>;
  if (!bot) return null;
  const guilds = bot.guilds ?? [];
  const guild = guilds.find((g) => g.id === bot.guild_id);
  return (
    <div className="grid gap-2 text-sm">
      <p className={bot.bot_online ? "text-state-available" : "text-muted"}>
        {bot.bot_online
          ? `${bot.bot_name ?? "The bot"} is online${guild ? ` in ${guild.name}` : ""}.`
          : `${bot.bot_name ?? "The bot"} is not online yet.`}
      </p>
      {guilds.length === 0 && bot.invite_url && (
        <p>
          <a className="btn-primary inline-block" href={bot.invite_url} target="_blank" rel="noreferrer">
            Invite the bot to your Discord server
          </a>
          <span className="ml-3 text-muted">Then come back and press Save.</span>
        </p>
      )}
      {guild && !bot.announce_channel_set && (
        <p className="text-muted">
          Pick the channel for new films and episodes in Anchorr
          {bot.dashboard_url ? (
            <>
              {" "}
              (
              <a className="underline" href={bot.dashboard_url} target="_blank" rel="noreferrer">
                open Anchorr
              </a>
              )
            </>
          ) : null}
          .
        </p>
      )}
      {guild && bot.announce_channel_set && bot.dashboard_url && (
        <p className="text-muted">
          Channels, roles and messages are set in{" "}
          <a className="underline" href={bot.dashboard_url} target="_blank" rel="noreferrer">
            Anchorr
          </a>
          .
        </p>
      )}
    </div>
  );
}

/** The Discord bot: one token here, the rest is set up for it. */
export function AnchorrSection() {
  const qc = useQueryClient();
  const { data } = useQuery({ queryKey: ["settings", "anchorr"], queryFn: () => unwrap(api.GET("/api/v1/anchorr")) });
  const [f, setF] = useState({ enabled: false, url: "http://anchorr:8282", public_url: "", discord_token: "", guild_id: "" });
  useEffect(() => {
    if (data)
      setF({
        enabled: data.enabled,
        url: data.url || "http://anchorr:8282",
        public_url: data.public_url,
        discord_token: "",
        guild_id: data.bot?.guild_id ?? "",
      });
  }, [data]);
  const save = useMutation({
    mutationFn: () =>
      unwrap(
        api.PUT("/api/v1/anchorr", {
          body: { ...f, discord_token: f.discord_token || null, guild_id: f.guild_id || null },
        }),
      ),
    onSuccess: (v) => {
      setF((x) => ({ ...x, discord_token: "" }));
      qc.setQueryData(["settings", "anchorr"], v);
    },
  });
  const text = (k: "url" | "public_url" | "discord_token") => (e: React.ChangeEvent<HTMLInputElement>) =>
    setF((x) => ({ ...x, [k]: e.target.value }));
  const guilds = data?.bot?.guilds ?? [];

  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        save.mutate();
      }}
      className="mt-6 grid gap-6"
    >
      <label className="inline-flex items-center gap-2 text-sm text-muted">
        <input
          type="checkbox"
          className="accent-accent"
          checked={f.enabled}
          onChange={(e) => setF((x) => ({ ...x, enabled: e.target.checked }))}
        />
        Run the Discord bot
      </label>
      {f.enabled && (
        <>
          <Field
            label="Discord bot token"
            hint="Discord Developer Portal → your application → Bot → Reset Token. Turn on Server Members Intent on the same page."
          >
            <input
              className="field data text-base"
              type="password"
              value={f.discord_token}
              onChange={text("discord_token")}
              placeholder={data?.enabled ? "Saved. Paste a new one to replace it" : "Paste the bot token"}
              autoComplete="off"
            />
          </Field>
          {guilds.length > 1 && (
            <Field label="Discord server">
              <select
                className="field text-base"
                value={f.guild_id}
                onChange={(e) => setF((x) => ({ ...x, guild_id: e.target.value }))}
              >
                <option value="">Choose a server</option>
                {guilds.map((g) => (
                  <option key={g.id} value={g.id}>
                    {g.name}
                  </option>
                ))}
              </select>
            </Field>
          )}
          <details className="text-sm">
            <summary className="cursor-pointer text-muted">Advanced</summary>
            <div className="mt-4 grid gap-6 sm:grid-cols-2">
              <Field label="Anchorr address" hint="Where Moka reaches the Anchorr container.">
                <input className="field data text-base" value={f.url} onChange={text("url")} placeholder="http://anchorr:8282" />
              </Field>
              <Field label="Moka's public address" hint="For links in Discord. Leave empty for none.">
                <input
                  className="field data text-base"
                  value={f.public_url}
                  onChange={text("public_url")}
                  placeholder="https://moka.example.com"
                />
              </Field>
            </div>
          </details>
        </>
      )}
      {data && <BotState view={data} />}
      <div className="flex flex-wrap items-center gap-3">
        <button type="submit" className="btn-primary" disabled={save.isPending}>
          {save.isPending ? "Connecting…" : "Save"}
        </button>
        <p role="status" className={`text-sm ${save.error ? "text-danger" : "text-state-available"}`}>
          {save.error?.message ?? (save.isSuccess ? "Saved." : "")}
        </p>
      </div>
    </form>
  );
}
