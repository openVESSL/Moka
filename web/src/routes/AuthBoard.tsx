import { useState, type FormEvent, type ReactNode } from "react";
import { Wordmark } from "../components/Wordmark";

/** Sign-in and first-run screens: one small card. */
export function AuthBoard({
  title,
  intro,
  submitLabel,
  onSubmit,
  passwordAutocomplete,
  footer,
}: {
  title: string;
  intro: ReactNode;
  submitLabel: string;
  onSubmit: (username: string, password: string) => Promise<void>;
  passwordAutocomplete: "current-password" | "new-password";
  footer?: ReactNode;
}) {
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(false);

  async function submit(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError(undefined);
    try {
      await onSubmit(username, password);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Something went wrong. Try again.");
      setBusy(false);
    }
  }

  return (
    <main className="grid min-h-dvh place-items-center bg-sunk px-4 py-10">
      <div className="w-full max-w-[26rem]">
        <Wordmark className="mb-6" />
        <form
          onSubmit={submit}
          className="card p-7 sm:p-8"
      >
          <h1 className="text-2xl font-bold">
            {title}
          </h1>
          <p className="mt-3 max-w-[42ch] text-[15px] leading-relaxed text-muted">{intro}</p>

          <div className="mt-6 grid gap-4">
            <label className="block">
              <span className="field-label">Name</span>
              <input
                className="field"
                value={username}
                onChange={(e) => setUsername(e.target.value)}
                autoComplete="username"
                autoCapitalize="none"
                spellCheck={false}
                required
                maxLength={64}
              />
            </label>
            <label className="block">
              <span className="field-label">Password</span>
              <input
                className="field"
                type="password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                autoComplete={passwordAutocomplete}
                required
                minLength={passwordAutocomplete === "new-password" ? 8 : undefined}
              />
            </label>
          </div>

          <div className="mt-6 flex flex-wrap items-center gap-x-4 gap-y-3">
            <button type="submit" className="btn-primary" disabled={busy}>
              {busy ? "Checking…" : submitLabel}
            </button>
            <p role="alert" className="text-sm text-danger">
              {error}
            </p>
            </div>
        </form>
        {footer && <div className="mt-6 text-sm text-faint">{footer}</div>}
      </div>
    </main>
  );
}
