import { useRouter } from "@tanstack/react-router";
import ePub, { type Book, type Location, type NavItem, type Rendition } from "epubjs";
import { ArrowLeft, ChevronLeft, ChevronRight, Minus, Plus } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { api, unwrap } from "../api/client";

const SAVE_AFTER_MS = 1500;
const SIZE_KEY = "moka.reader-size";
const SIZES = [80, 90, 100, 110, 125, 140, 160];

function storedSize(): number {
  try {
    const n = Number(localStorage.getItem(SIZE_KEY));
    return SIZES.includes(n) ? n : 100;
  } catch {
    return 100;
  }
}

function storeSize(n: number) {
  try {
    localStorage.setItem(SIZE_KEY, String(n));
  } catch {
    // private mode: the size just is not remembered
  }
}

/** Every entry of a table of contents, nested ones included. */
function flatten(items: NavItem[], depth = 0): { item: NavItem; depth: number }[] {
  return items.flatMap((item) => [{ item, depth }, ...flatten(item.subitems ?? [], depth + 1)]);
}

/** An EPUB read in the page, one screen at a time, starting where the reader last stopped. */
export default function ReaderPage({ fileId }: { fileId: number }) {
  const router = useRouter();
  const view = useRef<HTMLDivElement>(null);
  const rendition = useRef<Rendition | null>(null);
  const [toc, setToc] = useState<{ item: NavItem; depth: number }[]>([]);
  const [chapter, setChapter] = useState("");
  const [ready, setReady] = useState(false);
  const [error, setError] = useState<string>();
  const [size, setSize] = useState(storedSize);

  useEffect(() => {
    let book: Book | undefined;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const keys = (e: KeyboardEvent) => {
      if (e.key === "ArrowRight" || e.key === "PageDown") void rendition.current?.next();
      if (e.key === "ArrowLeft" || e.key === "PageUp") void rendition.current?.prev();
    };

    (async () => {
      const res = await fetch(`/api/v1/files/${fileId}/stream`);
      if (!res.ok) throw new Error(res.status === 404 ? "This book file is gone." : `The book did not load (${res.status}).`);
      const data = await res.arrayBuffer();
      const saved = await unwrap(api.GET("/api/v1/files/{id}/progress", { params: { path: { id: fileId } } })).catch(
        () => undefined,
      );
      if (cancelled || !view.current) return;
      book = ePub(data);
      const r = book.renderTo(view.current, {
        width: "100%",
        height: "100%",
        flow: "paginated",
        spread: "auto",
        minSpreadWidth: 1100,
        allowScriptedContent: false,
      });
      rendition.current = r;
      // the book's pages live in a frame: give them the app's colours and type
      r.themes.default({
        html: { background: "#0c0c0e !important" },
        body: { color: "#f4f4f5 !important", background: "#0c0c0e !important", "line-height": "1.65" },
        "p, li, span, div": { color: "inherit !important" },
        a: { color: "#a5a8fb !important" },
      });
      r.themes.fontSize(`${storedSize()}%`);
      r.on("keyup", keys);
      r.on("relocated", (loc: Location) => {
        const href = loc.start.href;
        setChapter(href);
        clearTimeout(timer);
        timer = setTimeout(() => {
          void api.PUT("/api/v1/files/{id}/progress", {
            params: { path: { id: fileId } },
            body: { position_seconds: 0, played: !!loc.atEnd, bookmark: loc.start.cfi },
          });
        }, SAVE_AFTER_MS);
      });
      book.loaded.navigation.then((nav) => !cancelled && setToc(flatten(nav.toc)));
      await r.display(saved?.bookmark ?? undefined).catch(() => r.display());
      if (!cancelled) setReady(true);
    })().catch((e: Error) => !cancelled && setError(e.message));

    document.addEventListener("keyup", keys);
    return () => {
      cancelled = true;
      clearTimeout(timer);
      document.removeEventListener("keyup", keys);
      rendition.current = null;
      book?.destroy();
    };
  }, [fileId]);

  const resize = (d: 1 | -1) => {
    const next = SIZES[Math.min(Math.max(SIZES.indexOf(size) + d, 0), SIZES.length - 1)];
    setSize(next);
    storeSize(next);
    rendition.current?.themes.fontSize(`${next}%`);
  };
  // the contents entry for the part on screen
  const current = toc.find((t) => chapter && t.item.href.split("#")[0] === chapter.split("#")[0]);

  return (
    <main className="flex h-[calc(100dvh-3.5rem)] flex-col md:h-dvh">
      <div className="flex h-14 shrink-0 items-center gap-2 border-b border-line px-3 sm:px-5">
        <button
          type="button"
          onClick={() => router.history.back()}
          className="grid size-10 place-items-center rounded-lg text-muted hover:bg-white/5 hover:text-fg"
          aria-label="Back"
        >
          <ArrowLeft aria-hidden className="size-5" />
        </button>
        {toc.length > 0 && (
          <select
            aria-label="Chapter"
            className="field ml-auto h-9 max-w-[16rem] min-w-0 py-0 text-sm"
            value={current?.item.href ?? ""}
            onChange={(e) => void rendition.current?.display(e.target.value)}
          >
            {!current && <option value="">Contents</option>}
            {toc.map(({ item, depth }) => (
              <option key={item.id + item.href} value={item.href}>
                {" ".repeat(depth)}
                {item.label.trim()}
              </option>
            ))}
          </select>
        )}
        <div className={`flex items-center gap-1 ${toc.length ? "" : "ml-auto"}`}>
          <button type="button" onClick={() => resize(-1)} className="player-btn" aria-label="Smaller text">
            <Minus aria-hidden className="size-4" />
          </button>
          <span className="data w-11 text-center text-xs text-faint">{size}%</span>
          <button type="button" onClick={() => resize(1)} className="player-btn" aria-label="Larger text">
            <Plus aria-hidden className="size-4" />
          </button>
        </div>
      </div>

      <div className="relative flex min-h-0 flex-1 items-stretch">
        <button
          type="button"
          onClick={() => void rendition.current?.prev()}
          className="hidden w-14 shrink-0 place-items-center text-faint hover:text-fg sm:grid"
          aria-label="Previous page"
        >
          <ChevronLeft aria-hidden className="size-6" />
        </button>
        <div ref={view} className="mx-auto w-full max-w-3xl min-w-0 flex-1 py-6" />
        <button
          type="button"
          onClick={() => void rendition.current?.next()}
          className="hidden w-14 shrink-0 place-items-center text-faint hover:text-fg sm:grid"
          aria-label="Next page"
        >
          <ChevronRight aria-hidden className="size-6" />
        </button>
        {!ready && !error && <div className="absolute inset-0 m-auto h-1/2 w-2/3 animate-pulse rounded bg-raised/50" />}
        {error && <p className="absolute inset-x-0 top-1/3 text-center text-danger">{error}</p>}
      </div>

      {/* phones: tap the halves of this bar to turn pages */}
      <div className="grid h-12 shrink-0 grid-cols-2 border-t border-line sm:hidden">
        <button type="button" onClick={() => void rendition.current?.prev()} className="text-muted" aria-label="Previous page">
          <ChevronLeft aria-hidden className="mx-auto size-5" />
        </button>
        <button type="button" onClick={() => void rendition.current?.next()} className="text-muted" aria-label="Next page">
          <ChevronRight aria-hidden className="mx-auto size-5" />
        </button>
      </div>
    </main>
  );
}
