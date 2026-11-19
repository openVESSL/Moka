/**
 * A grease-pencil loop drawn around something, the way a good take gets
 * circled on a camera report. Used for films already in the library.
 */
export function GreaseCircle({ className = "" }: { className?: string }) {
  return (
    <svg
      aria-hidden
      viewBox="0 0 120 60"
      preserveAspectRatio="none"
      className={`pointer-events-none absolute text-grease ${className}`}
    >
      <path
        d="M14 34c-6-14 18-27 49-27 30 0 52 8 50 22-2 15-28 25-57 24C27 52 6 44 9 29c2-9 15-17 33-20"
        fill="none"
        stroke="currentColor"
        strokeWidth="3.2"
        strokeLinecap="round"
        vectorEffect="non-scaling-stroke"
      />
    </svg>
  );
}
