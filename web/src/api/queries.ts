import { queryOptions, useMutation, useQueryClient } from "@tanstack/react-query";
import { api, unwrap } from "./client";

export const DISCOVER_LISTS = [
  { id: "trending", label: "Trending" },
  { id: "popular", label: "Popular" },
  { id: "now_playing", label: "In cinemas" },
  { id: "upcoming", label: "Coming soon" },
  { id: "top_rated", label: "All-time" },
] as const;
export type DiscoverList = (typeof DISCOVER_LISTS)[number]["id"];

export const authStatusQuery = queryOptions({
  queryKey: ["auth", "status"],
  queryFn: () => unwrap(api.GET("/api/v1/auth/status")),
  staleTime: Infinity,
});

export const meQuery = queryOptions({
  queryKey: ["auth", "me"],
  queryFn: () => unwrap(api.GET("/api/v1/me")),
  staleTime: 5 * 60_000,
  retry: false,
});

export const discoverQuery = (list: DiscoverList, page = 1) =>
  queryOptions({
    queryKey: ["catalog", "discover", list, page],
    queryFn: () => unwrap(api.GET("/api/v1/catalog/discover/{list}", { params: { path: { list }, query: { page } } })),
    staleTime: 10 * 60_000,
    retry: false,
  });

export const searchQuery = (q: string) =>
  queryOptions({
    queryKey: ["catalog", "search", q],
    queryFn: () => unwrap(api.GET("/api/v1/catalog/search", { params: { query: { q } } })),
    enabled: q.trim().length > 0,
    staleTime: 5 * 60_000,
    retry: false,
  });

export const movieQuery = (tmdbId: number) =>
  queryOptions({
    queryKey: ["catalog", "movie", tmdbId],
    queryFn: () =>
      unwrap(api.GET("/api/v1/catalog/movie/{tmdb_id}", { params: { path: { tmdb_id: tmdbId } } })),
    staleTime: 10 * 60_000,
    retry: false,
  });

export const requestsQuery = (status?: "pending" | "approved" | "declined") =>
  queryOptions({
    queryKey: ["requests", status ?? "all"],
    queryFn: () => unwrap(api.GET("/api/v1/requests", { params: { query: { status } } })),
  });

export const titlesQuery = queryOptions({
  queryKey: ["titles"],
  queryFn: () => unwrap(api.GET("/api/v1/titles")),
});

export const catalogSettingsQuery = queryOptions({
  queryKey: ["settings", "catalog"],
  queryFn: () => unwrap(api.GET("/api/v1/settings/catalog")),
});

export const quotaQuery = queryOptions({
  queryKey: ["settings", "requests"],
  queryFn: () => unwrap(api.GET("/api/v1/settings/requests")),
});

/** Requesting a film touches catalog annotations, the request list and titles. */
export function useRequestFilm() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (tmdbId: number) => unwrap(api.POST("/api/v1/requests", { body: { tmdb_id: tmdbId } })),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["catalog"] });
      qc.invalidateQueries({ queryKey: ["requests"] });
      qc.invalidateQueries({ queryKey: ["titles"] });
    },
  });
}

export function useDecideRequest() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, approve }: { id: number; approve: boolean }) =>
      approve
        ? unwrap(api.POST("/api/v1/requests/{id}/approve", { params: { path: { id } }, body: {} }))
        : unwrap(api.POST("/api/v1/requests/{id}/decline", { params: { path: { id } }, body: {} })),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["requests"] });
      qc.invalidateQueries({ queryKey: ["titles"] });
      qc.invalidateQueries({ queryKey: ["catalog"] });
    },
  });
}
