import { useState, type FormEvent, type ReactNode } from "react";
import { Wordmark } from "../components/Wordmark";

/**
 * Sign-in and first-run screens drawn as a clapper slate. The sticks stand
 * open while you fill it in and clap shut when the form succeeds.
 */
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
  const [clapped, setClapped] = useState(false);

  async function submit(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError(undefined);
    try {
      await onSubmit(username, password);
      setClapped(true);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Something went wrong. Try again.");
      setBusy(false);
    }
  }

  return (
    <main className="grid min-h-dvh place-items-center bg-slate-sunk px-4 py-10">
      <div className="w-full max-w-[34rem]">
        <Wordmark className="mb-8" />
        <div className="relative">
          {/* the sticks, hinged at the left */}
          <div
            aria-hidden
            className="sticks relative z-10 h-12 origin-[0.75rem_100%] rounded-t-[4px] shadow-[0_6px_10px_-6px_rgb(0_0_0/0.8)] transition-transform duration-[260ms] ease-[cubic-bezier(0.5,0,0.75,0)]"
            style={{ transform: clapped ? "rotate(0deg)" : "rotate(-11deg)" }}
          />
          <div aria-hidden className="sticks h-12 opacity-90 [background-position:18px_0]" />
          <form
            onSubmit={submit}
            className="rounded-b-[4px] border border-t-0 border-slate-line bg-slate px-6 pt-7 pb-8 sm:px-9"
          >
            <h1 className="font-slate text-4xl leading-none font-extrabold tracking-[-0.01em] uppercase sm:text-5xl">
              {title}
            </h1>
            <p className="mt-3 max-w-[42ch] text-[15px] leading-relaxed text-chalk-dim">{intro}</p>

            <div className="mt-8 grid gap-6 sm:grid-cols-2">
              <label className="block">
                <span className="slate-label">Name</span>
                <input
                  className="chalk-input"
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
                <span className="slate-label">Password</span>
                <input
                  className="chalk-input"
                  type="password"
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  autoComplete={passwordAutocomplete}
                  required
                  minLength={passwordAutocomplete === "new-password" ? 8 : undefined}
                />
              </label>
            </div>

            <div className="mt-8 flex flex-wrap items-center gap-x-6 gap-y-3">
              <button type="submit" className="btn-grease" disabled={busy}>
                {busy && !clapped ? "Checking…" : submitLabel}
              </button>
              <p role="alert" className="text-sm text-grease-hot">
                {error}
              </p>
            </div>
          </form>
        </div>
        {footer && <div className="mt-6 text-sm text-chalk-faint">{footer}</div>}
      </div>
    </main>
  );
}
