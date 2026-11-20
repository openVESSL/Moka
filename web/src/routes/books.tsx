import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { BookOpen, UserRound } from "lucide-react";
import { authorPhoto, bookCover } from "../api/client";
import { bookAuthorQuery, bookQuery, meQuery } from "../api/queries";
import { Facts } from "../components/Facts";
import { PosterGrid } from "../components/Posters";
import { RequestButton } from "../components/RequestButton";
import { TitlePanel } from "../components/TitlePanel";

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
          <div className="mt-7 flex flex-wrap items-center gap-x-8 gap-y-3">
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
