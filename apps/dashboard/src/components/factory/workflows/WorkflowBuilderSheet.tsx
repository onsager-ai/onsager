import { useMemo } from "react"
import { useQuery } from "@tanstack/react-query"
import { api } from "@/lib/api"
import { useIsMobile } from "@/hooks/use-mobile"
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
 * WorkflowBuilder inside a shadcn Sheet. Closes itself on successful create.
 */
export function WorkflowBuilderSheet({ open, onOpenChange }: WorkflowBuilderSheetProps) {
  const isMobile = useIsMobile()

  const { data: workspacesData } = useQuery({
    queryKey: ["workspaces"],
    queryFn: api.listWorkspaces,
    enabled: open,
    staleTime: 30_000,
  })
  const workspaces = workspacesData?.tenants ?? []
  const tenantId = workspaces[0]?.id ?? ""

  const { data: installsData } = useQuery({
    queryKey: ["workspace-installations", tenantId],
    queryFn: () => api.listWorkspaceInstallations(tenantId),
    enabled: open && !!tenantId,
    staleTime: 30_000,
  })
  const installations = useMemo(() => installsData?.installations ?? [], [installsData])

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent
        side={isMobile ? "bottom" : "right"}
        className={isMobile ? "h-[90dvh] rounded-t-xl" : "sm:max-w-xl"}
      >
        <SheetHeader>
          <SheetTitle>Create workflow</SheetTitle>
          <SheetDescription>
            Chat out the idea, then tap cards to tune.
          </SheetDescription>
        </SheetHeader>
        <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain px-4 pb-4">
          {tenantId ? (
            <WorkflowBuilder
              tenantId={tenantId}
              installations={installations}
              onCreated={() => onOpenChange(false)}
            />
          ) : (
            <p className="py-6 text-sm text-muted-foreground">
              Create a workspace first — workflows live inside a workspace.
            </p>
          )}
        </div>
      </SheetContent>
    </Sheet>
  )
}
