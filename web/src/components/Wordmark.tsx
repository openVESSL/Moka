/** Product name, set once here so a rename is one edit. */
export const PRODUCT_NAME = "Moka";

/** The name in text; no logo yet. */
export function Wordmark({ className = "" }: { className?: string }) {
  return <span className={`inline-block text-xl font-bold tracking-[-0.03em] ${className}`}>{PRODUCT_NAME}</span>;
}
