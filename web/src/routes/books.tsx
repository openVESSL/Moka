import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { BookOpen, Download, Headphones, UserRound } from "lucide-react";
import { api, authorPhoto, bookCover, unwrap } from "../api/client";
import { bookAuthorQuery, bookQuery, meQuery, titleFilesQuery } from "../api/queries";
import { Facts } from "../components/Facts";
import { usePlayer } from "../components/Player";
import { PosterGrid } from "../components/Posters";
import { RequestButton } from "../components/RequestButton";
import { TitlePanel } from "../components/TitlePanel";

const ACTION =
  "inline-flex h-11 items-center gap-2 rounded-full bg-fg px-5 font-semibold text-canvas transition hover:scale-[1.03]";

/** Read in the browser (EPUB) and download, once the ebook is in the library. */
function EbookActions({ titleId }: { titleId: number }) {
  const { data: files } = useQuery(titleFilesQuery(titleId));
  const isEpub = (path: string) => /\.(epub|kepub)$/i.test(path);
  const file = files?.find((f) => isEpub(f.path)) ?? files?.[0];
  if (!file) return null;
  const epub = isEpub(file.path);
  return (
    <>
      {epub && (
        <Link to="/read/$fileId" params={{ fileId: String(file.id) }} className={ACTION}>
          <BookOpen aria-hidden className="size-4" />
          Read
        </Link>
      )}
      <a href={`/api/v1/files/${file.id}/stream`} download className="btn">
        <Download aria-hidden className="size-4" />
        Download {file.path.split(".").pop()?.toUpperCase()}
      </a>
    </>
  );
}

/** Listen from where the last session stopped, once the audiobook is in the library. */
function AudiobookActions({ titleId, title, author, coverId }: { titleId: number; title: string; author: string; coverId?: number | null }) {
  const qc = useQueryClient();
  const player = usePlayer();
  const { data: files } = useQuery(titleFilesQuery(titleId));
  if (!files?.length) return null;
  const parts = [...files].sort((a, b) => a.path.localeCompare(b.path, undefined, { numeric: true }));
  const listen = async () => {
    const place = await qc
      .fetchQuery({
        queryKey: ["files", parts[0].id, "progress"],
        queryFn: () => unwrap(api.GET("/api/v1/files/{id}/progress", { params: { path: { id: parts[0].id } } })),
        staleTime: 0,
      })
      .catch(() => undefined);
    const at = Math.max(0, parts.findIndex((p) => String(p.id) === place?.bookmark));
    const startAt = place?.played ? 0 : place?.position_seconds;
    const queue = parts.map((p, i) => ({
      fileId: p.id,
      title: parts.length > 1 ? `${title} · ${i + 1}/${parts.length}` : title,
      artist: author,
      album: title,
      art: bookCover(coverId),
      book: { startAt: i === at ? startAt : undefined },
    }));
    player.play(queue, place?.played ? 0 : at);
  };
  return (
    <button type="button" onClick={() => void listen()} className={ACTION}>
      <Headphones aria-hidden className="size-4" />
      Listen
    </button>
  );
}

function Failed({ error }: { error: Error }) {
  return <p className="mx-auto max-w-xl px-4 pt-24 text-center text-danger">{error.message}</p>;
}

export function BookPage({ olid }: { olid: string }) {
  const { data: b, error, isPending } = useQuery(bookQuery(olid));
  const { data: me } = useQuery(meQuery);
  if (error) return <Failed error={error} />;
  if (isPending) return <div className="h-[40vh] animate-pulse bg-raised/50" />;

  return (
    <main>
      <section className="mx-auto grid max-w-[110rem] gap-8 px-4 pt-8 pb-8 sm:px-8 md:grid-cols-[minmax(0,15rem)_1fr] md:items-end">
        <div className="relative aspect-[2/3] overflow-hidden rounded-md bg-raised shadow-[0_24px_40px_-18px_rgb(0_0_0/0.95)]">
          <BookOpen aria-hidden className="absolute inset-0 m-auto size-10 text-faint" strokeWidth={1.5} />
          {b.cover_id && (
            <img
              src={bookCover(b.cover_id)}
              alt={`${b.title} cover`}
              className="relative size-full object-cover"
              onError={(e) => (e.currentTarget.style.display = "none")}
            />
          )}
        </div>
        <div className="min-w-0">
          <h1 className="max-w-[22ch] text-[clamp(2rem,4.5vw,3.5rem)] leading-[1.05] font-bold tracking-[-0.03em]">
            {b.title}
          </h1>
          <p className="mt-3 flex flex-wrap gap-x-3 text-lg text-muted">
            {b.authors.map((name, i) =>
              b.author_olids[i] ? (
                <Link key={name} to="/author/$olid" params={{ olid: b.author_olids[i] }} className="hover:text-white">
                  {name}
                </Link>
              ) : (
                <span key={name}>{name}</span>
              ),
            )}
          </p>
          <Facts className="mt-7 max-w-3xl" fields={[{ label: "First published", value: b.year, mono: true }]} />
          <div className="mt-7 flex flex-wrap items-center gap-3">
            {b.ebook?.state === "available" && <EbookActions titleId={b.ebook.title_id} />}
            {b.audiobook?.state === "available" && (
              <AudiobookActions titleId={b.audiobook.title_id} title={b.title} author={b.authors[0] ?? ""} coverId={b.cover_id} />
            )}
          </div>
          <div className="mt-5 flex flex-wrap items-center gap-x-8 gap-y-3">
            <RequestButton tmdbId={0} kind="ebook" olid={b.olid} local={b.ebook} label="ebook" />
            <RequestButton tmdbId={0} kind="audiobook" olid={b.olid} local={b.audiobook} label="audiobook" />
          </div>
        </div>
      </section>

      {(b.description || b.subjects.length > 0) && (
        <section className="mx-auto max-w-[110rem] px-4 sm:px-8">
          {b.description && <p className="max-w-[65ch] whitespace-pre-line text-muted">{b.description}</p>}
          {b.subjects.length > 0 && (
            <ul className="mt-5 flex max-w-3xl flex-wrap gap-2">
              {b.subjects.map((s) => (
                <li key={s} className="rounded-full border border-line px-3 py-1 text-xs text-muted">
                  {s}
                </li>
              ))}
            </ul>
          )}
        </section>
      )}

      {b.ebook && (
        <>
          <h2 className="field-label mx-auto mt-10 max-w-[110rem] px-4 sm:px-8">Ebook</h2>
          <TitlePanel titleId={b.ebook.title_id} permissions={me?.permissions ?? 0} />
        </>
      )}
      {b.audiobook && (
        <>
          <h2 className="field-label mx-auto mt-10 max-w-[110rem] px-4 sm:px-8">Audiobook</h2>
          <TitlePanel titleId={b.audiobook.title_id} permissions={me?.permissions ?? 0} />
        </>
      )}
    </main>
  );
}

export function BookAuthorPage({ olid }: { olid: string }) {
  const { data, error, isPending } = useQuery(bookAuthorQuery(olid));
  if (error) return <Failed error={error} />;
  if (isPending) return <div className="h-[40vh] animate-pulse bg-raised/50" />;
  const a = data.author;
  const lived = [a.birth_date, a.death_date].filter(Boolean).join(" – ");

  return (
    <main className="mx-auto max-w-[110rem] px-4 pt-8 sm:px-8">
      <div className="flex items-end gap-6">
        <div className="relative size-24 shrink-0 overflow-hidden rounded-full bg-raised sm:size-32">
          <UserRound aria-hidden className="absolute inset-0 m-auto size-8 text-faint" strokeWidth={1.5} />
          {a.photo_id && (
            <img
              src={authorPhoto(a.photo_id)}
              alt=""
              className="relative size-full object-cover"
              onError={(e) => (e.currentTarget.style.display = "none")}
            />
          )}
        </div>
        <div className="min-w-0">
          <h1 className="text-[clamp(2rem,4.5vw,3.5rem)] leading-[1.05] font-bold tracking-[-0.03em]">{a.name}</h1>
          {lived && <p className="data mt-2 text-sm text-faint">{lived}</p>}
        </div>
      </div>
      {a.bio && <p className="mt-6 max-w-[65ch] whitespace-pre-line text-muted">{a.bio}</p>}
      <section aria-labelledby="books" className="mt-12">
        <h2 id="books" className="mb-3 text-base font-semibold">
          Books
        </h2>
        {data.books.length ? (
          <PosterGrid
            items={data.books.map((book) => ({
              key: book.olid,
              title: book.title,
              year: book.year,
              kind: "book" as const,
              olid: book.olid,
              coverId: book.cover_id,
              state: book.ebook?.state ?? book.audiobook?.state,
            }))}
          />
        ) : (
          <p className="text-muted">No books listed.</p>
        )}
      </section>
    </main>
  );
}
