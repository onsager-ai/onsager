import { useMemo } from "react"
import { useQuery } from "@tanstack/react-query"
import { api } from "@/lib/api"
import { useIsMobile } from "@/hooks/use-mobile"
import { useOptionalActiveWorkspace } from "@/lib/workspace"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet"
import { WorkflowBuilder } from "./WorkflowBuilder"

export interface WorkflowBuilderSheetProps {
  open: boolean
  onOpenChange: (open: boolean) => void
}

/**
 * Thin wrapper that loads the active workspace's installations and mounts
 * WorkflowBuilder. On desktop it renders inside a centered modal Dialog; on
 * mobile it falls back to a bottom Sheet (a centered modal is wrong on small
 * screens per the dashboard-ui mobile-chrome rules). Closes itself on a
 * successful create.
 */
export function WorkflowBuilderSheet({ open, onOpenChange }: WorkflowBuilderSheetProps) {
  const isMobile = useIsMobile()
  const activeWorkspace = useOptionalActiveWorkspace()

  // Prefer the URL-scoped workspace (#166); fall back to the user's first
  // membership for routes mounted outside `/workspaces/:workspace/*` (e.g.
  // legacy bare paths still alive during the redirect window).
  const { data: workspacesData } = useQuery({
    queryKey: ["workspaces"],
    queryFn: api.listWorkspaces,
    enabled: open && !activeWorkspace,
    staleTime: 30_000,
  })
  const workspaces = workspacesData?.workspaces ?? []
  const workspaceId = activeWorkspace?.id ?? workspaces[0]?.id ?? ""

  const { data: installsData } = useQuery({
    queryKey: ["workspace-installations", workspaceId],
    queryFn: () => api.listWorkspaceInstallations(workspaceId),
    enabled: open && !!workspaceId,
    staleTime: 30_000,
  })
  const installations = useMemo(() => installsData?.installations ?? [], [installsData])

  const body = workspaceId ? (
    <WorkflowBuilder
      workspaceId={workspaceId}
      installations={installations}
      onCreated={() => onOpenChange(false)}
    />
  ) : (
    <p className="py-6 text-sm text-muted-foreground">
      Create a workspace first — workflows live inside a workspace.
    </p>
  )

  if (isMobile) {
    return (
      <Sheet open={open} onOpenChange={onOpenChange}>
        <SheetContent side="bottom" className="flex h-[90dvh] flex-col rounded-t-xl">
          <SheetHeader>
            <SheetTitle>Create workflow</SheetTitle>
            <SheetDescription>
              Pick a template, then tune the trigger, steps, and repositories.
            </SheetDescription>
          </SheetHeader>
          <div className="flex min-h-0 flex-1 flex-col px-4 pb-4">{body}</div>
        </SheetContent>
      </Sheet>
    )
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="flex max-h-[85dvh] flex-col gap-4 sm:max-w-2xl lg:max-w-3xl xl:max-w-4xl">
        <DialogHeader>
          <DialogTitle>Create workflow</DialogTitle>
          <DialogDescription>
            Pick a template, then tune the trigger, steps, and repositories.
          </DialogDescription>
        </DialogHeader>
        <div className="flex min-h-0 flex-1 flex-col">{body}</div>
      </DialogContent>
    </Dialog>
  )
}
