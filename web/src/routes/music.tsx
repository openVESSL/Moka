import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { Disc3, Pause, Play } from "lucide-react";
import { cover } from "../api/client";
import { albumQuery, albumTracksQuery, artistQuery, meQuery } from "../api/queries";
import { RequestButton } from "../components/RequestButton";
import { PosterGrid } from "../components/Posters";
import { Facts } from "../components/Facts";
import { usePlayer, type QueueItem } from "../components/Player";
import { TitlePanel } from "../components/TitlePanel";

function minutes(seconds?: number | null) {
  if (!seconds) return "";
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}

export function ArtistPage({ mbid }: { mbid: string }) {
  const { data, error, isPending } = useQuery(artistQuery(mbid));
  if (error) return <p className="mx-auto max-w-xl px-4 pt-24 text-center text-danger">{error.message}</p>;
  if (isPending) return <div className="h-[40vh] animate-pulse bg-raised/50" />;
  const studio = data.albums.filter((a) => a.studio);
  const other = data.albums.filter((a) => !a.studio);
  const toItems = (albums: typeof data.albums) =>
    albums.map((a) => ({
      key: a.mbid,
      title: a.title,
      year: a.year,
      mbid: a.mbid,
      kind: "album" as const,
      state: a.local?.state,
    }));

  return (
    <main className="mx-auto max-w-[110rem] px-4 pt-8 sm:px-8">
      <h1 className="text-[clamp(2rem,4.5vw,3.5rem)] leading-[1.05] font-bold tracking-[-0.03em]">
        {data.artist.name}
      </h1>
      <p className="mt-3 text-muted">
        {[data.artist.disambiguation, data.artist.kind, data.artist.country].filter(Boolean).join(" · ")}
      </p>
      <section aria-labelledby="albums" className="mt-12">
        <h2 id="albums" className="mb-3 text-base font-semibold">
          Albums
        </h2>
        {studio.length ? <PosterGrid items={toItems(studio)} /> : <p className="text-muted">No studio albums listed.</p>}
      </section>
      {other.length > 0 && (
        <section aria-labelledby="other" className="mt-8">
          <h2 id="other" className="mb-3 text-base font-semibold">
            Live, compilations and more
          </h2>
          <PosterGrid items={toItems(other)} />
        </section>
      )}
    </main>
  );
}

export function AlbumPage({ mbid }: { mbid: string }) {
  const { data: a, error, isPending } = useQuery(albumQuery(mbid));
  const { data: me } = useQuery(meQuery);
  const localId = a?.local?.title_id;
  const { data: owned } = useQuery({ ...albumTracksQuery(localId ?? 0), enabled: localId !== undefined });
  const player = usePlayer();
  if (error) return <p className="mx-auto max-w-xl px-4 pt-24 text-center text-danger">{error.message}</p>;
  if (isPending) return <div className="h-[40vh] animate-pulse bg-raised/50" />;
  const total = a.tracks.reduce((sum, t) => sum + (t.length_seconds ?? 0), 0);
  const discs = new Set(a.tracks.map((t) => t.disc)).size;
  // tracks in the library, keyed by their number on the album
  const files = new Map<number, number>();
  for (const t of owned ?? []) if (t.file_id != null && t.number != null) files.set(t.number, t.file_id);
  const queue: QueueItem[] = a.tracks.flatMap((t) => {
    const fileId = files.get(t.number);
    return fileId === undefined ? [] : [{ fileId, title: t.title, artist: a.artist, album: a.title, mbid: a.mbid }];
  });

  return (
    <main>
      <section className="mx-auto grid max-w-[110rem] gap-8 px-4 pt-8 pb-8 sm:px-8 md:grid-cols-[minmax(0,18rem)_1fr] md:items-end">
        <div className="relative aspect-square overflow-hidden rounded-md bg-raised shadow-[0_24px_40px_-18px_rgb(0_0_0/0.95)]">
          <Disc3 aria-hidden className="absolute inset-0 m-auto size-10 text-faint" strokeWidth={1.5} />
          <img
            src={cover(a.mbid)}
            alt={`${a.title} cover`}
            className="relative size-full object-cover"
            onError={(e) => (e.currentTarget.style.display = "none")}
          />
        </div>
        <div className="min-w-0">
          <h1 className="max-w-[18ch] text-[clamp(2rem,4.5vw,3.5rem)] leading-[1.05] font-bold tracking-[-0.03em]">
            {a.title}
          </h1>
          {a.artist_mbid ? (
            <Link to="/artist/$mbid" params={{ mbid: a.artist_mbid }} className="mt-3 inline-block text-lg text-muted hover:text-white">
              {a.artist}
            </Link>
          ) : (
            <p className="mt-3 text-lg text-muted">{a.artist}</p>
          )}
          <Facts
            className="mt-7 max-w-3xl"
            fields={[
              { label: "Year", value: a.year, mono: true },
              { label: "Tracks", value: a.tracks.length, mono: true },
              { label: "Discs", value: discs > 1 ? discs : undefined, mono: true },
              { label: "Length", value: total ? minutes(total) : undefined, mono: true },
            ]}
          />
          <div className="mt-7 flex flex-wrap items-center gap-3">
            {queue.length > 0 && (
              <button
                type="button"
                onClick={() => player.play(queue, 0)}
                className="inline-flex h-11 items-center gap-2 rounded-full bg-fg px-5 font-semibold text-canvas transition hover:scale-[1.03]"
              >
                <Play aria-hidden className="size-4" fill="currentColor" />
                Play
              </button>
            )}
            <RequestButton tmdbId={0} kind="album" mbid={a.mbid} local={a.local} />
          </div>
        </div>
      </section>

      {a.local && <TitlePanel titleId={a.local.title_id} permissions={me?.permissions ?? 0} />}

      <section aria-labelledby="tracks" className="mx-auto mt-8 max-w-[110rem] px-4 sm:px-8">
        <h2 id="tracks" className="field-label mb-3">
          Tracks
        </h2>
        <ol className="max-w-3xl border-t border-line">
          {a.tracks.map((t) => {
            const label = `${discs > 1 ? `${t.disc}.` : ""}${String(t.number).padStart(2, "0")}`;
            const fileId = files.get(t.number);
            const current = fileId !== undefined && player.current?.fileId === fileId;
            const pause = current && player.playing;
            return (
              <li key={t.number} className="group flex items-center gap-4 border-b border-line py-1.5">
                <span className="grid h-9 w-10 shrink-0 place-items-center">
                  {fileId === undefined ? (
                    <span className="data text-sm text-faint">{label}</span>
                  ) : (
                    <button
                      type="button"
                      onClick={() => (current ? player.toggle() : player.play(queue, queue.findIndex((q) => q.fileId === fileId)))}
                      className={`grid size-9 place-items-center rounded-full hover:bg-white/10 hover:text-fg ${current ? "text-accent" : "text-faint"}`}
                      aria-label={pause ? `Pause ${t.title}` : `Play ${t.title}`}
                    >
                      {pause ? (
                        <Pause aria-hidden className="size-4" fill="currentColor" />
                      ) : (
                        <>
                          <span className={`data text-sm group-hover:hidden ${current ? "hidden" : ""}`}>{label}</span>
                          <Play
                            aria-hidden
                            className={`size-4 translate-x-px group-hover:block ${current ? "block" : "hidden"}`}
                            fill="currentColor"
                          />
                        </>
                      )}
                    </button>
                  )}
                </span>
                <span className={`min-w-0 flex-1 truncate text-[17px] font-semibold ${current ? "text-accent" : ""}`}>{t.title}</span>
                <span className="data text-sm text-faint">{minutes(t.length_seconds)}</span>
              </li>
            );
          })}
        </ol>
      </section>
    </main>
  );
}
