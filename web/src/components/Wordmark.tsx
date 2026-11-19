/** Working name, set once here so a rename is one edit. */
export const PRODUCT_NAME = "Koda";

export function Wordmark({ className = "" }: { className?: string }) {
  return (
    <span className={`inline-flex items-center gap-2.5 ${className}`}>
      <svg aria-hidden viewBox="0 0 32 32" className="size-7">
        <g transform="rotate(-14 6 13)">
          <rect x="5" y="8" width="23" height="5" fill="var(--color-chalk)" />
          <path d="M9 8h4l-3 5H6zM17 8h4l-3 5h-4zM25 8h3v1l-2.4 4H22z" fill="var(--color-slate)" />
        </g>
        <rect x="5" y="14" width="22" height="13" rx="1" fill="var(--color-chalk)" />
      </svg>
      <span className="font-slate text-xl font-extrabold tracking-[0.06em] uppercase">{PRODUCT_NAME}</span>
    </span>
  );
}
