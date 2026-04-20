import { useState } from "react"
import { Link } from "react-router-dom"
import { Check, ChevronRight, Circle, X } from "lucide-react"
import { Button } from "@/components/ui/button"
import { useSetupProgress } from "@/hooks/useSetupProgress"

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
 */
export function SetupChecklist() {
  const {
    authed,
    loading,
    hasWorkspace,
    hasInstall,
    hasProject,
    complete,
    firstWorkspaceSlug,
  } = useSetupProgress()
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

  const steps: { done: boolean; title: string; href: string; active?: boolean }[] = [
    {
      done: hasWorkspace,
      title: "Create a workspace",
      href: "/workspaces?welcome=1",
    },
    {
      done: hasInstall,
      title: "Connect GitHub",
      href: firstWorkspaceSlug ? `/workspaces#${firstWorkspaceSlug}` : "/workspaces",
    },
    {
      done: hasProject,
      title: "Add a project",
      href: firstWorkspaceSlug ? `/workspaces#${firstWorkspaceSlug}` : "/workspaces",
    },
  ]
  const activeIndex = steps.findIndex((s) => !s.done)
  if (activeIndex >= 0) steps[activeIndex].active = true
  const doneCount = steps.filter((s) => s.done).length

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
              to={step.href}
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
