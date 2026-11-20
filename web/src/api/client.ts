import createClient from "openapi-fetch";
import type { components, paths } from "./schema";

export type Schemas = components["schemas"];
export type CatalogItem = Schemas["CatalogItem"];
export type CatalogMovie = Schemas["CatalogMovie"];
export type CatalogPage = Schemas["CatalogPage"];
export type Me = Schemas["Me"];
export type Request = Schemas["Request"];
export type Title = Schemas["Title"];

export const api = createClient<paths>({ baseUrl: "", credentials: "same-origin" });

/** Error with the server's message and machine code (`unauthorized`, `not_configured`, ...). */
export class ApiError extends Error {
  constructor(
    public status: number,
    public code: string,
    message: string,
  ) {
    super(message);
  }
}

/** Unwraps an openapi-fetch result, throwing ApiError on failure. */
export async function unwrap<T>(
  call: Promise<{ data?: T; error?: unknown; response: Response }>,
): Promise<T> {
  const { data, error, response } = await call;
  if (response.ok) return data as T;
  const body = (error ?? {}) as { error?: string; message?: string };
  throw new ApiError(
    response.status,
    body.error ?? "unknown",
    body.message ?? `Request failed (${response.status})`,
  );
}

/** Album cover (Cover Art Archive) served through our own cache. */
export function cover(mbid?: string | null) {
  return mbid ? `/api/v1/images/cover/${mbid}` : undefined;
}

/** Book cover (Open Library) served through our own cache. */
export function bookCover(id?: number | string | null) {
  return id ? `/api/v1/images/book/${id}` : undefined;
}

/** Author photo (Open Library) served through our own cache. */
export function authorPhoto(id?: number | null) {
  return id ? `/api/v1/images/author/${id}` : undefined;
}

/** URL of a TMDB image served through our own cache. */
export function img(size: "w185" | "w342" | "w500" | "w780" | "w1280" | "original", path?: string | null) {
  if (!path) return undefined;
  return `/api/v1/images/${size}/${path.replace(/^\//, "")}`;
}
