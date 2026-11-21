import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useState, type FormEvent, type ReactNode } from "react";
import { api, unwrap } from "../api/client";
import { catalogSettingsQuery, quotaQuery } from "../api/queries";
import { ApiKeysSection } from "./apikeys";
import { PlaybackSection } from "./playback";
import { AnchorrSection } from "./anchorr";
import { DownloadClientSection, IndexersSection, JellyfinSection, LibrarySection, RadarrSection, ScanPanel } from "./connectors";
import { MusicProfileSection, ProfileSection } from "./profile";
import { UsersSection } from "./users";
import { WebhooksSection } from "./webhooks";

export function SettingsPage() {
  return (
    <Page title="Settings">
      <CatalogSection />
      <Section
        title="Quality"
        intro="Which releases are acceptable, best first. The first acceptable one found is downloaded; better ones replace it until the stop point."
      >
        <ProfileSection />
        <MusicProfileSection />
      </Section>
      <Section
        title="Library"
        intro="Where finished films, shows and albums are placed. Keep them on the same disk as your downloads so imports are instant and take no extra space."
      >
        <LibrarySection />
        <ScanPanel />
      </Section>
      <Section
        title="Playback"
        intro="When a browser cannot play a film as it is, Moka converts it on the fly. A graphics card does this faster and keeps the processor free. In Docker, the card must be passed to the container. If it fails, Moka uses the processor."
      >
        <PlaybackSection />
      </Section>
      <Section title="People" intro="Everyone who can sign in, and what each person may do.">
        <UsersSection />
      </Section>
      <QuotaSection />
    </Page>
  );
}

export function ConnectionsPage() {
  return (
    <Page title="Connections">
      <Section
        title="Download clients"
        intro="qBittorrent or Transmission for torrents, SABnzbd for Usenet. Downloads are added under their own category so you can tell them apart."
      >
        <DownloadClientSection />
      </Section>
      <Section
        title="Indexers"
        intro="Torznab or Newznab search addresses, from your indexer or from an indexer manager. Films are searched on all of them at once."
      >
        <IndexersSection />
      </Section>
      <Section
        title="Jellyfin"
        intro="Keep watching in Jellyfin if you like. Moka tells it about new films right after import and brings back what people watched there."
      >
        <JellyfinSection />
      </Section>
      <Section
        title="Discord bot"
        intro="People search and request from your Discord server through Anchorr, and hear when their films and shows are ready. Connect it from Anchorr: turn on System > Experimental > Use Moka there, then Connections > Moka. Each person's Discord ID, limits and messages are set under Users."
      >
        <AnchorrSection />
      </Section>
      <Section
        title="Radarr"
        intro="Already use Radarr? Moka can sit in front of it: approved requests go to Radarr, which searches and downloads, and every film Radarr has shows up in Moka's library to watch."
      >
        <RadarrSection />
      </Section>
      <Section
        title="Notifications"
        intro="Tell other apps when a film or show is requested, approved, downloading, ready or stuck. Works with Discord webhooks and any service that accepts JSON. A subtitle app's Radarr or Sonarr webhook address (ending in /api/webhooks/radarr or /api/webhooks/sonarr) is told about each new film or episode."
      >
        <WebhooksSection />
      </Section>
      <Section
        title="API keys"
        intro="Let other apps connect to Moka. Apps that can talk to Radarr (indexer managers, request apps) can use this server's address and a key from here. A key acts as the admin who made it."
      >
        <ApiKeysSection />
      </Section>
    </Page>
  );
}

/** A narrow page with a small title, for settings-like screens. */
export function Page({ title, children }: { title: string; children: ReactNode }) {
  return (
    <main className="mx-auto max-w-3xl px-4 pt-10 pb-20 sm:px-8">
      <h1 className="text-3xl font-bold">{title}</h1>
      {children}
    </main>
  );
}

function Section({ title, intro, children }: { title: string; intro: ReactNode; children: ReactNode }) {
  return (
    <section className="mt-6 card p-6 sm:p-7">
      <h2 className="text-base font-semibold">{title}</h2>
      <p className="mt-1 max-w-[65ch] text-sm text-muted">{intro}</p>
      {children}
    </section>
  );
}

function SaveRow({ busy, error, saved, label = "Save" }: { busy: boolean; error?: string; saved: boolean; label?: string }) {
  return (
    <div className="mt-8 flex flex-wrap items-center gap-x-5 gap-y-2">
      <button type="submit" className="btn-primary" disabled={busy}>
        {busy ? "Saving…" : label}
      </button>
      <p role="status" aria-live="polite" className={`text-sm ${error ? "text-danger" : "text-state-available"}`}>
        {error ?? (saved ? "Saved." : "")}
      </p>
    </div>
  );
}

function CatalogSection() {
  const qc = useQueryClient();
  const { data } = useQuery(catalogSettingsQuery);
  const [key, setKey] = useState("");
  const [language, setLanguage] = useState("en-US");
  const [region, setRegion] = useState("");

  useEffect(() => {
    if (!data) return;
    setLanguage(data.language);
    setRegion(data.region ?? "");
  }, [data]);

  const save = useMutation({
    /** `null` keeps the saved key, `""` goes back to the built-in one. */
    mutationFn: (tmdbKey: string | null) =>
      unwrap(
        api.PUT("/api/v1/settings/catalog", {
          body: { tmdb_key: tmdbKey, language, region: region || null },
        }),
      ),
    onSuccess: (v) => {
      setKey("");
      qc.setQueryData(catalogSettingsQuery.queryKey, v);
      qc.invalidateQueries({ queryKey: ["catalog"] });
    },
  });

  function submit(e: FormEvent) {
    e.preventDefault();
    save.mutate(key || null);
  }

  return (
    <Section
      title="Catalog"
      intro={
        <>
          Film and show listings, posters and details come from TMDB and work out of the box. You can use your own
          TMDB API key or read access token instead.
        </>
      }
    >
      <form onSubmit={submit} className="mt-6 grid gap-6 sm:grid-cols-[2fr_1fr_1fr]">
        <label className="block">
          <span className="flex items-baseline justify-between gap-2">
            <span className="field-label">Own TMDB key (optional)</span>
            {data?.tmdb_key_set && (
              <button
                type="button"
                className="text-xs font-medium text-accent hover:text-accent-hot"
                disabled={save.isPending}
                onClick={() => save.mutate("")}
              >
                Use built-in key
              </button>
            )}
          </span>
          <input
            className="field data text-base"
            type="password"
            autoComplete="off"
            spellCheck={false}
            value={key}
            onChange={(e) => setKey(e.target.value)}
            placeholder={data?.tmdb_key_set ? "Your key is saved. Paste a new one to replace it" : "Using the built-in key"}
          />
        </label>
        <label className="block">
          <span className="field-label">Language</span>
          <input
            className="field data text-base"
            value={language}
            onChange={(e) => setLanguage(e.target.value)}
            placeholder="en-US"
            pattern="[a-z]{2}(-[A-Z]{2})?"
            title="Like en-US or ro-RO"
          />
        </label>
        <label className="block">
          <span className="field-label">Region</span>
          <input
            className="field data text-base"
            value={region}
            onChange={(e) => setRegion(e.target.value)}
            placeholder="Any"
            maxLength={2}
            title="Two-letter country code, used for release dates and what is in cinemas"
          />
        </label>
        <div className="sm:col-span-3">
          <SaveRow busy={save.isPending} error={save.error?.message} saved={save.isSuccess} />
        </div>
      </form>
    </Section>
  );
}

function QuotaSection() {
  const qc = useQueryClient();
  const { data } = useQuery(quotaQuery);
  const [limit, setLimit] = useState("");
  const [days, setDays] = useState("7");

  useEffect(() => {
    if (!data) return;
    setLimit(data.limit ? String(data.limit) : "");
    setDays(String(data.days));
  }, [data]);

  const save = useMutation({
    mutationFn: () =>
      unwrap(
        api.PUT("/api/v1/settings/requests", {
          body: { limit: limit ? Number(limit) : null, days: Number(days) || 7 },
        }),
      ),
    onSuccess: (v) => qc.setQueryData(quotaQuery.queryKey, v),
  });

  return (
    <Section
      title="Request limit"
      intro="How many films each person can request in a period. Admins and request managers are never limited."
    >
      <form
        onSubmit={(e) => {
          e.preventDefault();
          save.mutate();
        }}
        className="mt-6 flex flex-wrap items-end gap-x-4 gap-y-4"
      >
        <label className="block w-28">
          <span className="field-label">Films</span>
          <input
            className="field data"
            inputMode="numeric"
            value={limit}
            onChange={(e) => setLimit(e.target.value.replace(/\D/g, ""))}
            placeholder="No limit"
          />
        </label>
        <span className="pb-3 text-muted">every</span>
        <label className="block w-24">
          <span className="field-label">Days</span>
          <input
            className="field data"
            inputMode="numeric"
            value={days}
            onChange={(e) => setDays(e.target.value.replace(/\D/g, ""))}
          />
        </label>
        <div className="w-full">
          <SaveRow busy={save.isPending} error={save.error?.message} saved={save.isSuccess} />
        </div>
      </form>
    </Section>
  );
}
