import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useState, type FormEvent, type ReactNode } from "react";
import { api, unwrap } from "../api/client";
import { catalogSettingsQuery, quotaQuery } from "../api/queries";
import { ApiKeysSection } from "./apikeys";
import { PlaybackSection } from "./playback";
import { DownloadClientSection, IndexersSection, JellyfinSection, LibrarySection, RadarrSection, ScanPanel } from "./connectors";
import { ProfileSection } from "./profile";
import { UsersSection } from "./users";
import { WebhooksSection } from "./webhooks";

export function SettingsPage() {
  return (
    <main className="mx-auto max-w-[56rem] px-4 pt-12 sm:px-8">
      <h1 className="font-slate text-5xl leading-none font-extrabold uppercase">Settings</h1>
      <CatalogSection />
      <Section
        title="Quality"
        intro="Which releases are acceptable, best first. The first acceptable one found is downloaded; better ones replace it until the stop point."
      >
        <ProfileSection />
      </Section>
      <Section
        title="Library"
        intro="Where finished films are placed. Keep it on the same disk as your downloads so imports are instant and take no extra space."
      >
        <LibrarySection />
        <ScanPanel />
      </Section>
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
        intro="Keep watching in Jellyfin if you like. Koda tells it about new films right after import and brings back what people watched there."
      >
        <JellyfinSection />
      </Section>
      <Section
        title="Playback"
        intro="When a browser cannot play a film as it is, Koda converts it on the fly. A graphics card does this faster and keeps the processor free. In Docker, the card must be passed to the container. If it fails, Koda uses the processor."
      >
        <PlaybackSection />
      </Section>
      <Section
        title="Radarr"
        intro="Already use Radarr? Koda can sit in front of it: approved requests go to Radarr, which searches and downloads, and every film Radarr has shows up in Koda's library to watch."
      >
        <RadarrSection />
      </Section>
      <Section title="People" intro="Everyone who can sign in, and what each person may do.">
        <UsersSection />
      </Section>
      <Section
        title="Notifications"
        intro="Tell other apps when a film or show is requested, approved, downloading, ready or stuck. Works with Discord webhooks and any service that accepts JSON. A subtitle app's Radarr or Sonarr webhook address (ending in /api/webhooks/radarr or /api/webhooks/sonarr) is told about each new film or episode."
      >
        <WebhooksSection />
      </Section>
      <Section
        title="API keys"
        intro="Let other apps connect to Koda. Apps that can talk to Radarr (indexer managers, request apps) can use this server's address and a key from here. A key acts as the admin who made it."
      >
        <ApiKeysSection />
      </Section>
      <QuotaSection />
    </main>
  );
}

function Section({ title, intro, children }: { title: string; intro: ReactNode; children: ReactNode }) {
  return (
    <section className="mt-14 border-t border-slate-line pt-8">
      <h2 className="font-slate text-2xl font-extrabold uppercase">{title}</h2>
      <p className="mt-2 max-w-[60ch] text-chalk-dim">{intro}</p>
      {children}
    </section>
  );
}

function SaveRow({ busy, error, saved, label = "Save" }: { busy: boolean; error?: string; saved: boolean; label?: string }) {
  return (
    <div className="mt-8 flex flex-wrap items-center gap-x-5 gap-y-2">
      <button type="submit" className="btn-grease" disabled={busy}>
        {busy ? "Saving…" : label}
      </button>
      <p role="status" aria-live="polite" className={`text-sm ${error ? "text-grease-hot" : "text-tape-available"}`}>
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
    mutationFn: () =>
      unwrap(
        api.PUT("/api/v1/settings/catalog", {
          body: { tmdb_key: key || null, language, region: region || null },
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
    save.mutate();
  }

  return (
    <Section
      title="Catalog"
      intro={
        <>
          Film listings, posters and details come from TMDB. Create a free API key in your TMDB account settings
          and paste either the API key or the read access token here.
        </>
      }
    >
      <form onSubmit={submit} className="mt-6 grid gap-6 sm:grid-cols-[2fr_1fr_1fr]">
        <label className="block">
          <span className="slate-label">TMDB key</span>
          <input
            className="chalk-input data text-base"
            type="password"
            autoComplete="off"
            spellCheck={false}
            value={key}
            onChange={(e) => setKey(e.target.value)}
            placeholder={data?.tmdb_key_set ? "Saved. Paste a new one to replace it" : "Paste key or token"}
          />
        </label>
        <label className="block">
          <span className="slate-label">Language</span>
          <input
            className="chalk-input data text-base"
            value={language}
            onChange={(e) => setLanguage(e.target.value)}
            placeholder="en-US"
            pattern="[a-z]{2}(-[A-Z]{2})?"
            title="Like en-US or ro-RO"
          />
        </label>
        <label className="block">
          <span className="slate-label">Region</span>
          <input
            className="chalk-input data text-base uppercase"
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
          <span className="slate-label">Films</span>
          <input
            className="chalk-input data"
            inputMode="numeric"
            value={limit}
            onChange={(e) => setLimit(e.target.value.replace(/\D/g, ""))}
            placeholder="No limit"
          />
        </label>
        <span className="pb-3 text-chalk-dim">every</span>
        <label className="block w-24">
          <span className="slate-label">Days</span>
          <input
            className="chalk-input data"
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
