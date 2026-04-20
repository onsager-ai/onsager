import { useState } from "react"
import { Link } from "react-router-dom"
import { Check, ChevronRight, Circle, X } from "lucide-react"
import { Button } from "@/components/ui/button"
import type { SetupProgress } from "@/hooks/useSetupProgress"

// Session-scoped dismissal — the user chose to hide the checklist for this
// session; we still re-show it on the next session so unfinished setup isn't
// permanently invisible.
const DISMISS_KEY = "onsager.setup_checklist_dismissed"

/**
 * Sidebar-embedded onboarding checklist. Shown whenever the user has started
 * but not finished workspace setup; auto-hides once all three steps are done.
 * Complements the progressive nav disclosure: pre-workspace users only see
 * "Workspaces" in the sidebar, but once they've created a workspace the full
 * nav unlocks and this checklist takes over as outer-loop guidance.
 *
 * Progress is passed in from `AppSidebar` rather than read via the hook to
 * keep a single observer/render path across sidebar + checklist.
 */
export function SetupChecklist({ progress }: { progress: SetupProgress }) {
  const { authed, loading, hasWorkspace, hasInstall, hasProject, complete } =
    progress
  const [dismissed, setDismissed] = useState(
    () =>
      typeof window !== "undefined" &&
      window.sessionStorage.getItem(DISMISS_KEY) === "1",
  )

  if (!authed || loading || complete || dismissed) return null

  const dismiss = () => {
    setDismissed(true)
    if (typeof window !== "undefined") {
      window.sessionStorage.setItem(DISMISS_KEY, "1")
    }
  }

  // All steps link to /workspaces — the page owns the setup flow (create
  // workspace, connect GitHub App, pick a repo). Deep-linking to a specific
  // workspace would need the page to read `location.hash` and scroll, which
  // it doesn't today; plain routes stay honest.
  const steps: { done: boolean; title: string; active?: boolean }[] = [
    { done: hasWorkspace, title: "Create a workspace" },
    { done: hasInstall, title: "Connect GitHub" },
    { done: hasProject, title: "Add a project" },
  ]
  const activeIndex = steps.findIndex((s) => !s.done)
  if (activeIndex >= 0) steps[activeIndex].active = true
  const doneCount = steps.filter((s) => s.done).length
  // Zero-workspace users land on /workspaces?welcome=1 to get the full hero;
  // everyone else just needs the page itself.
  const href = hasWorkspace ? "/workspaces" : "/workspaces?welcome=1"

  return (
    <div className="mx-2 mb-2 rounded-md border bg-sidebar-accent/40 p-3">
      <div className="mb-2 flex items-center justify-between gap-2">
        <div>
          <p className="text-xs font-semibold">Getting started</p>
          <p className="text-[10px] text-muted-foreground">
            {doneCount} of {steps.length} complete
          </p>
        </div>
        <Button
          variant="ghost"
          size="icon"
          onClick={dismiss}
          aria-label="Dismiss checklist"
          className="h-6 w-6 text-muted-foreground"
        >
          <X className="h-3.5 w-3.5" />
        </Button>
      </div>
      <ol className="space-y-1">
        {steps.map((step, i) => (
          <li key={i}>
            <Link
              to={href}
              className={
                "group flex items-center gap-2 rounded px-1.5 py-1 text-xs transition-colors " +
                (step.active
                  ? "bg-primary/10 text-foreground"
                  : "text-muted-foreground hover:bg-sidebar-accent")
              }
            >
              {step.done ? (
                <Check className="h-3.5 w-3.5 shrink-0 text-primary" />
              ) : (
                <Circle
                  className={
                    "h-3.5 w-3.5 shrink-0 " +
                    (step.active ? "text-primary" : "text-muted-foreground")
                  }
                />
              )}
              <span
                className={
                  "flex-1 " +
                  (step.done ? "line-through opacity-60" : "")
                }
              >
                {step.title}
              </span>
              {step.active && (
                <ChevronRight className="h-3.5 w-3.5 shrink-0 text-primary opacity-0 transition-opacity group-hover:opacity-100" />
              )}
            </Link>
          </li>
        ))}
      </ol>
    </div>
  )
}
