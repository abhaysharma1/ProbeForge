import { type FormEvent, useState } from "react";
import { ApiError } from "./api";

interface LoginViewProps {
  readonly onLogin: (email: string, password: string) => Promise<void>;
}

export function LoginView({ onLogin }: LoginViewProps) {
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setError(null);
    setSubmitting(true);
    try {
      await onLogin(email, password);
    } catch (loginError) {
      const message =
        loginError instanceof ApiError
          ? loginError.message
          : "Sign in failed. Check the API connection and try again.";
      setError(message);
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <main className="grid min-h-screen bg-pf-background lg:grid-cols-[minmax(0,1fr)_minmax(440px,0.8fr)]">
      <section className="hidden flex-col justify-between border-r border-pf-border px-12 py-10 lg:flex xl:px-16">
        <div className="flex items-center gap-3">
          <span
            aria-hidden="true"
            className="grid size-9 place-items-center rounded-md bg-pf-accent text-[13px] font-semibold tracking-[0.08em] text-white"
          >
            PF
          </span>
          <span className="text-[15px] font-semibold tracking-[-0.01em]">ProbeForge</span>
        </div>

        <div className="max-w-xl">
          <p className="mb-4 font-mono text-[11px] font-medium tracking-[0.14em] text-pf-text-muted">
            EVIDENCE-GATED TESTING
          </p>
          <h1 className="max-w-lg text-[32px] font-semibold leading-[1.16] tracking-[-0.025em] text-pf-text">
            Review the plan before anything touches your system.
          </h1>
          <p className="mt-5 max-w-md text-[14px] leading-6 text-pf-text-secondary">
            ProbeForge inspects repositories read-only, proposes vetted vulnerability and load
            tests, and requires explicit approval and target authorization before execution.
          </p>
          <dl className="mt-10 grid max-w-md grid-cols-2 gap-px overflow-hidden rounded-lg border border-pf-border bg-pf-border">
            <div className="bg-pf-surface px-4 py-4">
              <dt className="text-[11px] font-medium text-pf-text-muted">Execution model</dt>
              <dd className="mt-1 text-[13px] text-pf-text">Approval-gated</dd>
            </div>
            <div className="bg-pf-surface px-4 py-4">
              <dt className="text-[11px] font-medium text-pf-text-muted">Repository access</dt>
              <dd className="mt-1 text-[13px] text-pf-text">Read-only</dd>
            </div>
          </dl>
        </div>

        <p className="max-w-md text-[11px] leading-5 text-pf-text-muted">
          Sessions use an HttpOnly cookie. Sign-in credentials are verified with Argon2id and are
          never stored in browser storage.
        </p>
      </section>

      <section className="flex min-h-screen items-center justify-center px-6 py-10 sm:px-10">
        <div className="w-full max-w-[380px]">
          <div className="mb-10 flex items-center gap-3 lg:hidden">
            <span
              aria-hidden="true"
              className="grid size-8 place-items-center rounded-md bg-pf-accent text-[12px] font-semibold tracking-[0.08em] text-white"
            >
              PF
            </span>
            <span className="text-[15px] font-semibold tracking-[-0.01em]">ProbeForge</span>
          </div>

          <h2 className="text-[26px] font-semibold tracking-[-0.02em]">Sign in</h2>
          <p className="mt-2 text-[13px] leading-5 text-pf-text-secondary">
            Use the account provisioned by your ProbeForge operator.
          </p>

          <form className="mt-8 space-y-5" onSubmit={handleSubmit} noValidate>
            <div>
              <label htmlFor="email" className="mb-1.5 block text-[12px] font-medium text-pf-text">
                Email
              </label>
              <input
                id="email"
                name="email"
                type="email"
                autoComplete="email"
                required
                value={email}
                onChange={(event) => setEmail(event.target.value)}
                className="h-10 w-full rounded-md border border-pf-border-strong bg-pf-surface px-3 text-[14px] text-pf-text shadow-[0_1px_1px_rgba(32,34,31,0.03)] transition-colors placeholder:text-pf-text-muted hover:border-pf-text-muted focus:border-pf-accent focus:outline-none"
                placeholder="you@example.com"
              />
            </div>

            <div>
              <label
                htmlFor="password"
                className="mb-1.5 block text-[12px] font-medium text-pf-text"
              >
                Password
              </label>
              <input
                id="password"
                name="password"
                type="password"
                autoComplete="current-password"
                required
                value={password}
                onChange={(event) => setPassword(event.target.value)}
                className="h-10 w-full rounded-md border border-pf-border-strong bg-pf-surface px-3 text-[14px] text-pf-text shadow-[0_1px_1px_rgba(32,34,31,0.03)] transition-colors placeholder:text-pf-text-muted hover:border-pf-text-muted focus:border-pf-accent focus:outline-none"
                placeholder="Your password"
              />
            </div>

            {error ? (
              <p
                role="alert"
                aria-live="polite"
                className="rounded-md border border-[#e8c9c4] bg-[#fbf1ef] px-3 py-2 text-[12px] leading-5 text-pf-error"
              >
                {error}
              </p>
            ) : null}

            <button
              type="submit"
              disabled={submitting || email.trim() === "" || password === ""}
              className="inline-flex h-10 w-full items-center justify-center rounded-md bg-pf-accent px-4 text-[13px] font-medium text-white transition-colors hover:bg-pf-accent-hover disabled:cursor-not-allowed disabled:opacity-55"
            >
              {submitting ? "Signing in..." : "Sign in"}
            </button>
          </form>

          <p className="mt-6 border-t border-pf-border pt-5 text-[11px] leading-5 text-pf-text-muted">
            Five failed attempts in fifteen minutes lock sign-in for that account and address.
            Contact your operator if you need an account.
          </p>
        </div>
      </section>
    </main>
  );
}
