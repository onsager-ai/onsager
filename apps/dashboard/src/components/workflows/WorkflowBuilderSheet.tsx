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
        <SheetContent side="bottom" className="h-[90dvh] rounded-t-xl">
          <SheetHeader>
            <SheetTitle>Create workflow</SheetTitle>
            <SheetDescription>
              Chat out the idea, then tap cards to tune.
            </SheetDescription>
          </SheetHeader>
          <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain px-4 pb-4">
            {body}
          </div>
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
            Chat out the idea, then tap cards to tune.
          </DialogDescription>
        </DialogHeader>
        <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain">
          {body}
        </div>
      </DialogContent>
    </Dialog>
  )
}
