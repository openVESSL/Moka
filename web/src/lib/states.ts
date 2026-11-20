/** How each title state is named and which color it gets. */
export type TitleState =
  | "requested"
  | "wanted"
  | "searching"
  | "grabbed"
  | "downloading"
  | "importing"
  | "available";

export const STATE_LABEL: Record<TitleState, string> = {
  requested: "Requested",
  wanted: "Wanted",
  searching: "Searching",
  grabbed: "Sent to client",
  downloading: "Downloading",
  importing: "Importing",
  available: "In library",
};

/** One line on what happens next, in plain words. */
export const STATE_HINT: Record<TitleState, string> = {
  requested: "Waiting for someone to approve it.",
  wanted: "Approved. Looking for a release that fits your quality profile.",
  searching: "Asking your indexers right now.",
  grabbed: "A release was picked and handed to your download client.",
  downloading: "Your download client is fetching it.",
  importing: "Download finished. Moving it into your library.",
  available: "Ready to play.",
};

export type StateTone = "requested" | "wanted" | "working" | "available";

export function toneFor(state: string): StateTone {
  switch (state) {
    case "requested":
      return "requested";
    case "wanted":
      return "wanted";
    case "available":
      return "available";
    default:
      return "working";
  }
}

export function isTitleState(s: string): s is TitleState {
  return s in STATE_LABEL;
}

export function stateLabel(s: string) {
  return isTitleState(s) ? STATE_LABEL[s] : s;
}

export function runtime(minutes?: number | null) {
  if (!minutes) return undefined;
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  return h ? `${h}h ${String(m).padStart(2, "0")}m` : `${m}m`;
}

export function ago(unixSeconds: number) {
  const s = Math.max(0, Date.now() / 1000 - unixSeconds);
  if (s < 60) return "just now";
  if (s < 3600) return `${Math.floor(s / 60)} min ago`;
  if (s < 86400) return `${Math.floor(s / 3600)} h ago`;
  return `${Math.floor(s / 86400)} d ago`;
}
