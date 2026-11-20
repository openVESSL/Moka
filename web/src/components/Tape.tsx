import { stateLabel, tapeFor } from "../lib/states";

const DOT = {
  requested: "bg-state-requested",
  wanted: "bg-state-wanted",
  working: "bg-state-working",
  available: "bg-state-available",
} as const;

/** A small state badge: a colored dot and the state's name. */
export function Tape({ state, className = "" }: { state: string; className?: string }) {
  return (
    <span
      className={`inline-flex items-center gap-1.5 rounded-full border border-white/10 bg-sunk/85 px-2 py-0.5 text-[11px] leading-4 font-medium text-fg backdrop-blur ${className}`}
    >
      <span aria-hidden className={`size-1.5 rounded-full ${DOT[tapeFor(state)]}`} />
      {stateLabel(state)}
    </span>
  );
}
