import { Link } from "react-router-dom"
import { Button } from "@/components/ui/button"

// Stub landing page (#399 owns the full hero). This file exists so the
// factory-metaphor sentence from #408 location 4 has a concrete home;
// route is wired but the unauthenticated entry-point redesign is the
// scope of spec #399.
export function LandingPage() {
  return (
    <main className="mx-auto flex min-h-screen max-w-4xl flex-col gap-12 px-6 py-16">
      <header className="flex flex-col gap-4">
        <h1 className="text-4xl font-bold tracking-tight">Onsager</h1>
        <p className="max-w-2xl text-lg text-muted-foreground">
          Governed workflows for AI-augmented engineering work. When agents
          write code, ship safely. Open source, runs anywhere.
        </p>
        {/* Spec #407 trust-strip — locked copy. The link points at the
            public Dogfood showcase, the live reference for the
            Onsager-managing-Onsager claim. */}
        <p className="text-sm text-muted-foreground">
          <Link
            to="/showcase/dogfood"
            className="font-medium text-foreground underline-offset-4 hover:underline"
          >
            Onsager is the factory that builds Onsager — see live runs ↗
          </Link>
        </p>
      </header>

      <section className="flex flex-col gap-3">
        <h2 className="text-2xl font-semibold tracking-tight">
          Who this is for
        </h2>
        <p className="max-w-2xl text-muted-foreground">
          Staff and principal engineers who already use AI coding agents on
          personal or side projects, and want guardrails for what those
          agents land.
        </p>
      </section>

      <section className="flex flex-col gap-4">
        <h2 className="text-2xl font-semibold tracking-tight">What you do</h2>
        <ol className="flex flex-col gap-3 text-muted-foreground">
          <li>
            <span className="font-semibold text-foreground">
              ① Describe the policy in plain English.
            </span>{" "}
            The assistant proposes a workflow draft. You review the shape
            before anything is created. Think of each policy as a production
            line: an order comes in, the line moves the order through QC
            checkpoints, the line outputs a finished product (a merged PR,
            a triaged issue, a release note).
          </li>
          <li>
            <span className="font-semibold text-foreground">
              ② Bind the draft to a real repo.
            </span>{" "}
            Workspace, GitHub install, and project link up only when you
            choose to make the draft real — not as up-front setup.
          </li>
          <li>
            <span className="font-semibold text-foreground">
              ③ Watch runs land.
            </span>{" "}
            Every run shows what each stage verified, what it changed,
            and what it produced.
          </li>
        </ol>
      </section>

      <footer className="flex flex-wrap gap-3">
        <Button render={<Link to="/login" />}>Get started</Button>
        <Button
          variant="outline"
          render={
            <a
              href="https://github.com/onsager-ai/onsager"
              target="_blank"
              rel="noreferrer"
            />
          }
        >
          View on GitHub
        </Button>
      </footer>
    </main>
  )
}
