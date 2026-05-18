import { useEffect, useState } from "react"
import { OnsagerLogo } from "@/components/layout/OnsagerLogo"

// Spec #407 — public, unauthenticated Dogfood showcase. The shape comes
// from `GET /api/showcase/dogfood`; we don't reuse the dashboard's typed
// `api` helper because that suite assumes authenticated routes and would
// surface this surface in the auth probe path. The page renders
// pre-auth from a top-level App.tsx route, so we fetch with plain
// `fetch` to keep the dependency surface minimal.

interface StageInfo {
  index: number
  executor_kind: string
}

interface RunStage {
  index: number
  executor_kind: string
  status: "passed" | "failed" | "blocked" | "pending"
}

interface RunLink {
  number: number
  url: string | null
}

interface Run {
  id: string
  status: "passed" | "failed" | "blocked" | "pending"
  stages: RunStage[]
  spec: RunLink | null
  pr: RunLink | null
  started_at: string
  updated_at: string
}

interface ShowcaseStats {
  specs_shipped: number
  prs_merged: number
  verify_gates_passed: number
}

interface ShowcaseResponse {
  enabled: boolean
  workflow: {
    name: string
    stage_count: number
    stages: StageInfo[]
  } | null
  runs: Run[]
  stats_7d: ShowcaseStats
  last_activity_at: string | null
  is_quiet: boolean
  generated_at: string
}

const REPO_URL = "https://github.com/onsager-ai/onsager"

export function ShowcaseDogfoodPage() {
  const [data, setData] = useState<ShowcaseResponse | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    fetch("/api/showcase/dogfood")
      .then((r) => r.json())
      .then((body: ShowcaseResponse) => {
        if (!cancelled) setData(body)
      })
      .catch((e: unknown) => {
        if (!cancelled) {
          const msg = e instanceof Error ? e.message : String(e)
          setError(msg)
        }
      })
    return () => {
      cancelled = true
    }
  }, [])

  return (
    <main className="mx-auto flex min-h-screen max-w-4xl flex-col gap-12 px-6 py-12">
      <header className="flex items-center justify-between">
        <a href="/" className="flex items-center gap-2 text-foreground">
          <OnsagerLogo />
          <span className="text-sm font-semibold">Onsager</span>
        </a>
        <a
          href="/login"
          className="text-sm text-muted-foreground hover:text-foreground"
        >
          Sign in →
        </a>
      </header>

      <section className="flex flex-col gap-4">
        <h1 className="text-3xl font-bold tracking-tight">
          Onsager builds Onsager.
        </h1>
        {/* Spec #407 locked copy. Don't rewrite without amending the spec. */}
        <p className="max-w-2xl text-base text-muted-foreground">
          This page is produced by Onsager. The factory below ships the code
          that built this page.
        </p>
        <WorkflowStrip workflow={data?.workflow ?? null} />
      </section>

      {error ? (
        <p className="text-sm text-destructive">
          Couldn&apos;t load runs: {error}. The factory itself is fine — this is
          just the projection.
        </p>
      ) : null}

      <StatsStrip stats={data?.stats_7d} />

      <section className="flex flex-col gap-4">
        <h2 className="text-xl font-semibold tracking-tight">
          Live runs from this factory.
        </h2>
        <RunsList data={data} />
      </section>

      <footer className="flex flex-col gap-1 border-t pt-6 text-sm text-muted-foreground">
        <p>
          Want this for your team? Self-host:{" "}
          <a
            href={REPO_URL}
            className="text-foreground underline-offset-4 hover:underline"
            target="_blank"
            rel="noreferrer"
          >
            github.com/onsager-ai/onsager
          </a>{" "}
          · Cloud:{" "}
          <a
            href="/signup"
            className="text-foreground underline-offset-4 hover:underline"
          >
            app.onsager.ai/signup
          </a>
          .
        </p>
      </footer>
    </main>
  )
}

function WorkflowStrip({
  workflow,
}: {
  workflow: ShowcaseResponse["workflow"]
}) {
  if (!workflow) {
    return (
      <div className="h-16 animate-pulse rounded-md border bg-muted/30" />
    )
  }
  return (
    <div className="flex w-full items-center gap-2 overflow-x-auto rounded-md border bg-muted/20 p-3">
      {workflow.stages.map((stage, i) => (
        <div key={stage.index} className="flex items-center gap-2">
          {i > 0 ? (
            <span aria-hidden className="text-muted-foreground">
              →
            </span>
          ) : null}
          <span className="whitespace-nowrap rounded border bg-background px-2 py-1 text-xs font-medium">
            Stage {stage.index} · {stage.executor_kind}
          </span>
        </div>
      ))}
    </div>
  )
}

function StatsStrip({ stats }: { stats: ShowcaseStats | undefined }) {
  const counters: { label: string; value: number | string }[] = [
    {
      label: "Specs shipped this week",
      value: stats?.specs_shipped ?? "—",
    },
    {
      label: "PRs merged",
      value: stats?.prs_merged ?? "—",
    },
    {
      label: "Verify gates passed",
      value: stats?.verify_gates_passed ?? "—",
    },
  ]
  return (
    <section className="grid grid-cols-1 gap-3 sm:grid-cols-3">
      {counters.map((c) => (
        <div
          key={c.label}
          className="flex flex-col gap-1 rounded-md border bg-muted/20 p-4"
        >
          <span className="text-xs uppercase tracking-wide text-muted-foreground">
            {c.label}
          </span>
          <span className="text-2xl font-bold tabular-nums">{c.value}</span>
        </div>
      ))}
    </section>
  )
}

function RunsList({ data }: { data: ShowcaseResponse | null }) {
  if (!data) {
    return (
      <ul className="flex flex-col gap-2">
        {[0, 1, 2].map((i) => (
          <li
            key={i}
            className="h-14 animate-pulse rounded-md border bg-muted/30"
          />
        ))}
      </ul>
    )
  }
  if (!data.enabled) {
    return (
      <p className="text-sm text-muted-foreground">
        The Dogfood workflow is not yet configured on this deployment. The
        live reference will appear here once it&apos;s wired up.
      </p>
    )
  }
  if (data.runs.length === 0) {
    return (
      <p className="text-sm text-muted-foreground">
        No runs in the spine yet. The first run will appear here.
      </p>
    )
  }
  return (
    <>
      <ul className="flex flex-col gap-3">
        {data.runs.map((run) => (
          <RunRow key={run.id} run={run} />
        ))}
      </ul>
      {data.is_quiet ? <QuietFooter at={data.last_activity_at} /> : null}
    </>
  )
}

function RunRow({ run }: { run: Run }) {
  return (
    <li className="flex flex-col gap-2 rounded-md border bg-card p-4">
      <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1 text-sm">
        <StatusPill status={run.status} />
        {run.spec ? <SpecLink spec={run.spec} /> : null}
        {run.pr ? <PrLink pr={run.pr} /> : null}
        <span className="ml-auto text-xs text-muted-foreground tabular-nums">
          {formatRelative(run.updated_at)}
        </span>
      </div>
      <div className="flex flex-wrap items-center gap-1.5">
        {run.stages.map((s) => (
          <StagePill key={s.index} stage={s} />
        ))}
      </div>
    </li>
  )
}

function StatusPill({ status }: { status: Run["status"] }) {
  const styles: Record<Run["status"], string> = {
    passed: "bg-emerald-500/15 text-emerald-700 dark:text-emerald-300",
    failed: "bg-destructive/15 text-destructive",
    blocked: "bg-amber-500/15 text-amber-700 dark:text-amber-300",
    pending: "bg-muted text-muted-foreground",
  }
  return (
    <span
      className={`rounded px-2 py-0.5 text-xs font-semibold uppercase tracking-wide ${styles[status]}`}
    >
      {status}
    </span>
  )
}

function StagePill({ stage }: { stage: RunStage }) {
  const styles: Record<RunStage["status"], string> = {
    passed: "border-emerald-500/40 bg-emerald-500/10",
    failed: "border-destructive/50 bg-destructive/10",
    blocked: "border-amber-500/50 bg-amber-500/10",
    pending: "border-border bg-muted/40",
  }
  return (
    <span
      className={`whitespace-nowrap rounded border px-1.5 py-0.5 text-[11px] ${styles[stage.status]}`}
      title={`Stage ${stage.index} · ${stage.executor_kind} · ${stage.status}`}
    >
      {stage.index}·{stage.executor_kind}
    </span>
  )
}

function SpecLink({ spec }: { spec: RunLink }) {
  const label = `Spec #${spec.number}`
  if (spec.url) {
    return (
      <a
        href={spec.url}
        target="_blank"
        rel="noreferrer"
        className="font-medium underline-offset-4 hover:underline"
      >
        {label}
      </a>
    )
  }
  return <span className="font-medium">{label}</span>
}

function PrLink({ pr }: { pr: RunLink }) {
  const label = `PR #${pr.number}`
  if (pr.url) {
    return (
      <a
        href={pr.url}
        target="_blank"
        rel="noreferrer"
        className="text-muted-foreground underline-offset-4 hover:text-foreground hover:underline"
      >
        {label}
      </a>
    )
  }
  return <span className="text-muted-foreground">{label}</span>
}

function QuietFooter({ at }: { at: string | null }) {
  // Spec #407 quiet-week copy. We fill the timestamp into the placeholder
  // the spec template leaves blank.
  const rel = at ? formatRelative(at) : "never"
  return (
    <p className="mt-2 text-xs text-muted-foreground">
      Last activity: {rel}. We don&apos;t run agents on weekends.
    </p>
  )
}

function formatRelative(iso: string): string {
  const then = new Date(iso).getTime()
  if (Number.isNaN(then)) return iso
  const now = Date.now()
  const seconds = Math.floor((now - then) / 1000)
  if (seconds < 60) return "just now"
  const minutes = Math.floor(seconds / 60)
  if (minutes < 60) return `${minutes}m ago`
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return `${hours}h ago`
  const days = Math.floor(hours / 24)
  if (days < 30) return `${days}d ago`
  const months = Math.floor(days / 30)
  return `${months}mo ago`
}
