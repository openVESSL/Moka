import type { ReactNode } from "react";

export type SlateField = { label: string; value: ReactNode; mono?: boolean };

/** A row of small labelled facts. Fields with no value are left out. */
export function SlateFields({ fields, className = "" }: { fields: SlateField[]; className?: string }) {
  const shown = fields.filter((f) => f.value !== undefined && f.value !== null && f.value !== "");
  if (!shown.length) return null;
  return (
    <dl className={`flex flex-wrap gap-x-6 gap-y-2 ${className}`}>
      {shown.map((f) => (
        <div key={f.label} className="min-w-0">
          <dt className="text-xs text-faint">{f.label}</dt>
          <dd className={`truncate text-sm font-medium ${f.mono ? "data" : ""}`}>{f.value}</dd>
        </div>
      ))}
    </dl>
  );
}
