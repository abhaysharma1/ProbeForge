import { JOB_GOALS, type JobGoal } from "@probe-forge/contracts";

const GOAL_COPY: Record<JobGoal, { readonly title: string; readonly description: string }> = {
  vulnerability_testing: {
    title: "Vulnerability testing",
    description:
      "Read-only repository analysis proposes security tests from a vetted catalog. You review the plan, select individual tests, and authorize the target before anything runs.",
  },
  load_testing: {
    title: "Load testing",
    description:
      "Endpoint-aware scenarios are generated from the repository. You approve the scenario, load profile, schedule, and target before any traffic is sent.",
  },
};

export default function App() {
  return (
    <div className="flex min-h-screen flex-col bg-slate-950 text-slate-100">
      <header className="border-b border-slate-800/80 bg-slate-950/80">
        <div className="mx-auto flex w-full max-w-5xl flex-wrap items-center justify-between gap-4 px-6 py-6">
          <div>
            <h1 className="text-2xl font-semibold tracking-tight text-white">ProbeForge</h1>
            <p className="mt-1 text-sm text-slate-400">
              AI-planned, approval-gated security and load testing.
            </p>
          </div>
          <span className="rounded-full border border-cyan-500/40 bg-cyan-500/10 px-3 py-1 text-xs font-medium text-cyan-300">
            Phase 1 &middot; local skeleton
          </span>
        </div>
      </header>

      <main className="mx-auto w-full max-w-5xl flex-1 px-6 py-12">
        <section aria-labelledby="goals-heading">
          <h2
            id="goals-heading"
            className="text-xs font-semibold uppercase tracking-[0.2em] text-slate-400"
          >
            Two job goals
          </h2>
          <div className="mt-5 grid gap-5 md:grid-cols-2">
            {JOB_GOALS.map((goal) => (
              <article
                key={goal}
                className="rounded-2xl border border-slate-800 bg-slate-900/60 p-6 shadow-lg shadow-slate-950/40"
              >
                <h3 className="text-lg font-semibold text-white">{GOAL_COPY[goal].title}</h3>
                <code className="mt-2 inline-block rounded bg-slate-800 px-2 py-1 text-xs text-cyan-300">
                  {goal}
                </code>
                <p className="mt-4 text-sm leading-6 text-slate-300">
                  {GOAL_COPY[goal].description}
                </p>
              </article>
            ))}
          </div>
        </section>

        <section
          aria-labelledby="safety-heading"
          className="mt-10 rounded-2xl border border-amber-500/30 bg-amber-500/5 p-6"
        >
          <h2 id="safety-heading" className="text-sm font-semibold text-amber-200">
            Safety boundary
          </h2>
          <p className="mt-2 text-sm leading-6 text-slate-300">
            Nothing runs without explicit plan approval and target authorization. Repository
            analysis never executes project code, installs dependencies, or starts the inspected
            application.
          </p>
        </section>
      </main>

      <footer className="border-t border-slate-800/80 px-6 py-5">
        <p className="mx-auto max-w-5xl text-xs text-slate-500">
          ProbeForge &middot; Phase 1 scaffold &middot; this page does not execute tests.
        </p>
      </footer>
    </div>
  );
}
