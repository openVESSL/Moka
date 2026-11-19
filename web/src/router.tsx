import type { QueryClient } from "@tanstack/react-query";
import { createRootRouteWithContext, createRoute, createRouter, Outlet, redirect } from "@tanstack/react-router";
import { DISCOVER_LISTS, authStatusQuery, meQuery, type DiscoverList } from "./api/queries";
import { LoginPage, SetupPage } from "./routes/auth";
import { DiscoverPage, SearchPage } from "./routes/discover";
import { LibraryPage } from "./routes/library";
import { MoviePage } from "./routes/movie";
import { RequestsPage } from "./routes/requests";
import { SettingsPage } from "./routes/settings";
import { Shell } from "./routes/Shell";
import { WatchPage } from "./routes/watch";

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
  validateSearch: (s: Record<string, unknown>): { list?: DiscoverList } => (isList(s.list) ? { list: s.list } : {}),
  component: function Discover() {
    const { list } = discover.useSearch();
    return <DiscoverPage list={list ?? "trending"} />;
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
const settings = createRoute({ getParentRoute: () => app, path: "/settings", component: SettingsPage });

const tree = root.addChildren([setup, login, app.addChildren([discover, search, movie, requests, library, settings, watch])]);

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
