import type { ComponentType } from "react"
import { Building2, GitBranch, Package, Sparkles } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Card, CardContent } from "@/components/ui/card"

/**
 * Hero shown when the user has zero workspaces. Walks them through the
 * three-step setup (create workspace → connect GitHub → pick a repo) and
 * opens the creation dialog on click.
 */
export function WorkspaceOnboarding({ onCreate }: { onCreate: () => void }) {
  return (
    <Card className="overflow-hidden">
      <CardContent className="space-y-6 p-6 md:p-8">
        <div className="space-y-2">
          <div className="inline-flex items-center gap-2 rounded-full bg-primary/10 px-3 py-1 text-xs font-medium text-primary">
            <Sparkles className="h-3 w-3" />
            Welcome to Onsager
          </div>
          <h2 className="text-xl font-semibold md:text-2xl">
            Let's set up your first workspace
          </h2>
          <p className="max-w-prose text-sm text-muted-foreground">
            A workspace is where Onsager runs agent sessions against your
            GitHub repositories. It owns your GitHub App installations, the
            projects you onboard, and the credentials your agents use.
          </p>
        </div>

        <ol className="grid gap-4 md:grid-cols-3">
          <Step
            n={1}
            title="Create a workspace"
            description="Name it after your team, org, or personal scope. You can create more later."
            icon={Building2}
            active
          />
          <Step
            n={2}
            title="Connect GitHub"
            description="Install the Onsager GitHub App on a user or org you own."
            icon={GitBranch}
          />
          <Step
            n={3}
            title="Add a project"
            description="Pick a repo the App can see. Agent sessions run against it."
            icon={Package}
          />
        </ol>

        <div className="flex flex-wrap items-center gap-3">
          <Button onClick={onCreate} size="lg">
            Create workspace
          </Button>
          <p className="text-xs text-muted-foreground">
            Takes under a minute.
          </p>
        </div>
      </CardContent>
    </Card>
  )
}

function Step({
  n,
  title,
  description,
  icon: Icon,
  active,
}: {
  n: number
  title: string
  description: string
  icon: ComponentType<{ className?: string }>
  active?: boolean
}) {
  return (
    <li
      className={
        "rounded-md border p-4 " +
        (active ? "border-primary/40 bg-primary/5" : "bg-muted/30")
      }
    >
      <div className="flex items-center gap-2">
        <span
          className={
            "flex h-6 w-6 items-center justify-center rounded-full text-xs font-semibold " +
            (active
              ? "bg-primary text-primary-foreground"
              : "bg-muted text-muted-foreground")
          }
        >
          {n}
        </span>
        <Icon className="h-4 w-4 text-muted-foreground" />
        <p className="text-sm font-medium">{title}</p>
      </div>
      <p className="mt-2 text-xs text-muted-foreground">{description}</p>
    </li>
  )
}
