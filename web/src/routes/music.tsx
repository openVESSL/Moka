import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { Disc3 } from "lucide-react";
import { cover } from "../api/client";
import { albumQuery, artistQuery, meQuery } from "../api/queries";
import { RequestButton } from "../components/RequestButton";
import { Roll } from "../components/Roll";
import { SlateFields } from "../components/SlateFields";
import { TitlePanel } from "../components/TitlePanel";

function minutes(seconds?: number | null) {
  if (!seconds) return "";
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}

export function ArtistPage({ mbid }: { mbid: string }) {
  const { data, error, isPending } = useQuery(artistQuery(mbid));
  if (error) return <p className="mx-auto max-w-xl px-4 pt-24 text-center text-grease-hot">{error.message}</p>;
  if (isPending) return <div className="h-[40vh] animate-pulse bg-slate-raised/50" />;
  const studio = data.albums.filter((a) => a.studio);
  const other = data.albums.filter((a) => !a.studio);
  const roll = (albums: typeof data.albums) =>
    albums.map((a) => ({
      key: a.mbid,
      title: a.title,
      year: a.year,
      mbid: a.mbid,
      kind: "album" as const,
      state: a.local?.state,
    }));

  return (
    <main className="mx-auto max-w-[110rem] px-4 pt-16 sm:px-8">
      <h1 className="font-slate text-[clamp(2.75rem,7vw,5.5rem)] leading-[0.88] font-extrabold tracking-[-0.02em] uppercase">
        {data.artist.name}
      </h1>
      <p className="mt-3 text-chalk-dim">
        {[data.artist.disambiguation, data.artist.kind, data.artist.country].filter(Boolean).join(" · ")}
      </p>
      <section aria-labelledby="albums" className="mt-12">
        <h2 id="albums" className="mb-6 border-b border-slate-line pb-3 font-slate text-2xl font-extrabold uppercase">
          Albums
        </h2>
        {studio.length ? <Roll numbered={false} items={roll(studio)} /> : <p className="text-chalk-dim">No studio albums listed.</p>}
      </section>
      {other.length > 0 && (
        <section aria-labelledby="other" className="mt-14">
          <h2 id="other" className="mb-6 border-b border-slate-line pb-3 font-slate text-2xl font-extrabold uppercase">
            Live, compilations and more
          </h2>
          <Roll numbered={false} items={roll(other)} />
        </section>
      )}
    </main>
  );
}

export function AlbumPage({ mbid }: { mbid: string }) {
  const { data: a, error, isPending } = useQuery(albumQuery(mbid));
  const { data: me } = useQuery(meQuery);
  if (error) return <p className="mx-auto max-w-xl px-4 pt-24 text-center text-grease-hot">{error.message}</p>;
  if (isPending) return <div className="h-[40vh] animate-pulse bg-slate-raised/50" />;
  const total = a.tracks.reduce((sum, t) => sum + (t.length_seconds ?? 0), 0);
  const discs = new Set(a.tracks.map((t) => t.disc)).size;

  return (
    <main>
      <section className="mx-auto grid max-w-[110rem] gap-10 px-4 pt-16 pb-10 sm:px-8 md:grid-cols-[minmax(0,18rem)_1fr] md:items-end">
        <div className="relative aspect-square overflow-hidden rounded-[3px] bg-slate-raised shadow-[0_24px_40px_-18px_rgb(0_0_0/0.95)]">
          <Disc3 aria-hidden className="absolute inset-0 m-auto size-10 text-chalk-faint" strokeWidth={1.5} />
          <img
            src={cover(a.mbid)}
            alt={`${a.title} cover`}
            className="relative size-full object-cover"
            onError={(e) => (e.currentTarget.style.display = "none")}
          />
        </div>
        <div className="min-w-0">
          <h1 className="max-w-[18ch] font-slate text-[clamp(2.5rem,6vw,5rem)] leading-[0.9] font-extrabold tracking-[-0.02em] uppercase">
            {a.title}
          </h1>
          {a.artist_mbid ? (
            <Link to="/artist/$mbid" params={{ mbid: a.artist_mbid }} className="mt-3 inline-block text-lg text-chalk-dim hover:text-white">
              {a.artist}
            </Link>
          ) : (
            <p className="mt-3 text-lg text-chalk-dim">{a.artist}</p>
          )}
          <SlateFields
            className="mt-7 max-w-3xl"
            fields={[
              { label: "Year", value: a.year, mono: true },
              { label: "Tracks", value: a.tracks.length, mono: true },
              { label: "Discs", value: discs > 1 ? discs : undefined, mono: true },
              { label: "Length", value: total ? minutes(total) : undefined, mono: true },
            ]}
          />
          <div className="mt-7">
            <RequestButton tmdbId={0} kind="album" mbid={a.mbid} local={a.local} />
          </div>
        </div>
      </section>

      {a.local && <TitlePanel titleId={a.local.title_id} permissions={me?.permissions ?? 0} />}

      <section aria-labelledby="tracks" className="mx-auto mt-14 max-w-[110rem] px-4 sm:px-8">
        <h2 id="tracks" className="slate-label mb-3">
          Tracks
        </h2>
        <ol className="max-w-3xl border-t border-slate-line">
          {a.tracks.map((t) => (
            <li key={t.number} className="flex items-baseline gap-4 border-b border-slate-line py-2.5">
              <span className="w-10 shrink-0 data text-sm text-chalk-faint">
                {discs > 1 ? `${t.disc}.` : ""}
                {String(t.number).padStart(2, "0")}
              </span>
              <span className="min-w-0 flex-1 truncate font-slate text-[17px] font-semibold">{t.title}</span>
              <span className="data text-sm text-chalk-faint">{minutes(t.length_seconds)}</span>
            </li>
          ))}
        </ol>
      </section>
    </main>
  );
}
