import type { ReactNode } from "react";

export type SlateField = { label: string; value: ReactNode; mono?: boolean };

/**
 * The ruled boxes of a clapper slate: a condensed label printed in the
 * corner, the value chalked in big. Fields with no value are left out.
 */
export function SlateFields({ fields, className = "" }: { fields: SlateField[]; className?: string }) {
  const shown = fields.filter((f) => f.value !== undefined && f.value !== null && f.value !== "");
  if (!shown.length) return null;
  return (
    <dl
      className={`grid grid-flow-col auto-cols-fr border-y border-chalk/25 divide-x divide-chalk/25 ${className}`}
    >
      {shown.map((f) => (
        <div key={f.label} className="min-w-0 px-3 py-2 first:pl-0">
          <dt className="slate-label text-[10px]">{f.label}</dt>
          <dd className={`mt-0.5 truncate text-lg font-semibold ${f.mono ? "data" : "font-slate"}`}>{f.value}</dd>
        </div>
      ))}
    </dl>
  );
}
