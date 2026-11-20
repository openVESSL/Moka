import type { QueryClient } from "@tanstack/react-query";
import { createRootRouteWithContext, createRoute, createRouter, Outlet, redirect } from "@tanstack/react-router";
import { lazy, Suspense } from "react";
import { DISCOVER_LISTS, authStatusQuery, meQuery, type DiscoverList, type MediaKind } from "./api/queries";
import { LoginPage, SetupPage } from "./routes/auth";
import { DiscoverPage, SearchPage } from "./routes/discover";
import { LibraryPage } from "./routes/library";
import { MoviePage } from "./routes/movie";
import { ShowPage } from "./routes/show";
import { AlbumPage, ArtistPage } from "./routes/music";
import { BookAuthorPage, BookPage } from "./routes/books";
import { RequestsPage } from "./routes/requests";
import { ConnectionsPage, SettingsPage } from "./routes/settings";
import { AboutPage } from "./routes/about";
import { Shell } from "./routes/Shell";
import { WatchPage } from "./routes/watch";

// the EPUB reader is large and only needed when reading
const ReaderPage = lazy(() => import("./routes/read"));

type Ctx = { queryClient: QueryClient };

const root = createRootRouteWithContext<Ctx>()({ component: Outlet });

const setup = createRoute({
  getParentRoute: () => root,
  path: "/setup",
  beforeLoad: async ({ context }) => {
    const s = await context.queryClient.fetchQuery(authStatusQuery);
    if (!s.needs_setup) throw redirect({ to: "/login" });
  },
  component: SetupPage,
});

const login = createRoute({
  getParentRoute: () => root,
  path: "/login",
  beforeLoad: async ({ context }) => {
    const s = await context.queryClient.fetchQuery(authStatusQuery);
    if (s.needs_setup) throw redirect({ to: "/setup" });
  },
  component: LoginPage,
});

/** Everything behind sign-in. */
const app = createRoute({
  getParentRoute: () => root,
  id: "app",
  beforeLoad: async ({ context }) => {
    const s = await context.queryClient.fetchQuery(authStatusQuery);
    if (s.needs_setup) throw redirect({ to: "/setup" });
    const me = await context.queryClient.fetchQuery(meQuery).catch(() => null);
    if (!me) throw redirect({ to: "/login" });
  },
  component: Shell,
});

const isList = (v: unknown): v is DiscoverList => DISCOVER_LISTS.some((l) => l.id === v);

const discover = createRoute({
  getParentRoute: () => app,
  path: "/",
  validateSearch: (s: Record<string, unknown>): { list?: DiscoverList; kind?: MediaKind } => ({
    ...(isList(s.list) ? { list: s.list } : {}),
    ...(s.kind === "show" ? { kind: "show" as const } : {}),
  }),
  component: function Discover() {
    const { list, kind } = discover.useSearch();
    return <DiscoverPage list={list} kind={kind ?? "movie"} />;
  },
});

const search = createRoute({
  getParentRoute: () => app,
  path: "/search",
  validateSearch: (s: Record<string, unknown>): { q: string } => ({ q: typeof s.q === "string" ? s.q : "" }),
  component: function Search() {
    const { q } = search.useSearch();
    return <SearchPage q={q} />;
  },
});

const movie = createRoute({
  getParentRoute: () => app,
  path: "/movie/$tmdbId",
  component: function Movie() {
    const { tmdbId } = movie.useParams();
    return <MoviePage key={tmdbId} tmdbId={Number(tmdbId)} />;
  },
});

const show = createRoute({
  getParentRoute: () => app,
  path: "/show/$tmdbId",
  component: function Show() {
    const { tmdbId } = show.useParams();
    return <ShowPage key={tmdbId} tmdbId={Number(tmdbId)} />;
  },
});

const artist = createRoute({
  getParentRoute: () => app,
  path: "/artist/$mbid",
  component: function Artist() {
    const { mbid } = artist.useParams();
    return <ArtistPage key={mbid} mbid={mbid} />;
  },
});

const album = createRoute({
  getParentRoute: () => app,
  path: "/album/$mbid",
  component: function Album() {
    const { mbid } = album.useParams();
    return <AlbumPage key={mbid} mbid={mbid} />;
  },
});

const book = createRoute({
  getParentRoute: () => app,
  path: "/book/$olid",
  component: function Book() {
    const { olid } = book.useParams();
    return <BookPage key={olid} olid={olid} />;
  },
});

const author = createRoute({
  getParentRoute: () => app,
  path: "/author/$olid",
  component: function Author() {
    const { olid } = author.useParams();
    return <BookAuthorPage key={olid} olid={olid} />;
  },
});

const requests = createRoute({
  getParentRoute: () => app,
  path: "/requests",
  validateSearch: (s: Record<string, unknown>): { status?: "pending" | "approved" | "declined" } =>
    s.status === "approved" || s.status === "declined" || s.status === "pending" ? { status: s.status } : {},
  component: function Requests() {
    const { status } = requests.useSearch();
    return <RequestsPage filter={status ?? "pending"} />;
  },
});

const library = createRoute({ getParentRoute: () => app, path: "/library", component: LibraryPage });
const watch = createRoute({
  getParentRoute: () => app,
  path: "/watch/$fileId",
  component: function Watch() {
    const { fileId } = watch.useParams();
    return <WatchPage fileId={Number(fileId)} />;
  },
});
const read = createRoute({
  getParentRoute: () => app,
  path: "/read/$fileId",
  component: function Read() {
    const { fileId } = read.useParams();
    return (
      <Suspense fallback={<div className="h-[40vh] animate-pulse bg-raised/50" />}>
        <ReaderPage key={fileId} fileId={Number(fileId)} />
      </Suspense>
    );
  },
});
const settings = createRoute({ getParentRoute: () => app, path: "/settings", component: SettingsPage });
const connections = createRoute({ getParentRoute: () => app, path: "/connections", component: ConnectionsPage });
const about = createRoute({ getParentRoute: () => app, path: "/about", component: AboutPage });

const tree = root.addChildren([
  setup,
  login,
  app.addChildren([discover, search, movie, show, artist, album, book, author, requests, library, settings, connections, about, watch, read]),
]);

export function makeRouter(queryClient: QueryClient) {
  return createRouter({
    routeTree: tree,
    context: { queryClient },
    defaultPreload: "intent",
    scrollRestoration: true,
  });
}

declare module "@tanstack/react-router" {
  interface Register {
    router: ReturnType<typeof makeRouter>;
  }
}
