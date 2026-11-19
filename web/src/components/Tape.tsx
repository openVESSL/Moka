import { stateLabel, tapeFor } from "../lib/states";

const TAPE_BG = {
  requested: "bg-tape-requested",
  wanted: "bg-tape-wanted",
  working: "bg-tape-working",
  available: "bg-tape-available",
} as const;

/**
 * A strip of paper camera tape with the state written on it. Torn ends,
 * a slight lean, ink-dark text: the way crews label cases and mags.
 */
export function Tape({ state, className = "" }: { state: string; className?: string }) {
  return (
    <span
      className={`inline-block px-2.5 py-[3px] font-slate text-[11px] leading-none font-bold tracking-[0.12em] text-ink uppercase shadow-[0_1px_2px_rgb(0_0_0/0.35)] ${TAPE_BG[tapeFor(state)]} ${className}`}
      style={{
        clipPath:
          "polygon(0 8%, 3% 0, 97% 4%, 100% 0, 99% 50%, 100% 96%, 96% 100%, 4% 94%, 0 100%, 1% 50%)",
      }}
    >
      {stateLabel(state)}
    </span>
  );
}
