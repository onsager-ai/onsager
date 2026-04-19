import { type FormEvent, useMemo, useState, type ReactElement } from "react"
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { api, type TaskRequest } from "@/lib/api"
import { useNodes } from "@/hooks/useNodes"
import { useIsMobile } from "@/hooks/use-mobile"
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetTitle,
  SheetTrigger,
} from "@/components/ui/sheet"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Textarea } from "@/components/ui/textarea"

interface CreateSessionSheetProps {
  children?: ReactElement
  open?: boolean
  onOpenChange?: (open: boolean) => void
}

export function CreateSessionSheet({ children, open: openProp, onOpenChange }: CreateSessionSheetProps) {
  const [internalOpen, setInternalOpen] = useState(false)
  const open = openProp ?? internalOpen
  const setOpen = onOpenChange ?? setInternalOpen
  const [prompt, setPrompt] = useState("")
  const [nodeId, setNodeId] = useState("")
  const [workingDir, setWorkingDir] = useState("")
  const [workspaceId, setWorkspaceId] = useState("")
  const [projectId, setProjectId] = useState("")
  const isMobile = useIsMobile()
  const queryClient = useQueryClient()
  const { data: nodesData } = useNodes()
  const onlineNodes = nodesData?.nodes.filter((n) => n.status === "online") ?? []

  const { data: wsData } = useQuery({
    queryKey: ["workspaces"],
    queryFn: api.listWorkspaces,
  })
  const workspaces = wsData?.tenants ?? []

  const { data: projectsData } = useQuery({
    queryKey: ["all-projects"],
    queryFn: api.listAllProjects,
  })

  const visibleProjects = useMemo(() => {
    const all = projectsData?.projects ?? []
    return workspaceId ? all.filter((p) => p.tenant_id === workspaceId) : []
  }, [projectsData, workspaceId])

  const mutation = useMutation({
    mutationFn: (task: TaskRequest) => api.createTask(task),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["sessions"] })
      setOpen(false)
      setPrompt("")
      setNodeId("")
      setWorkingDir("")
      setWorkspaceId("")
      setProjectId("")
    },
  })

  function handleSubmit(e: FormEvent) {
    e.preventDefault()
    if (!prompt.trim()) return
    const validNodeId = onlineNodes.some((n) => n.id === nodeId) ? nodeId : ""
    const validProjectId = visibleProjects.some((p) => p.id === projectId)
      ? projectId
      : ""
    mutation.mutate({
      prompt: prompt.trim(),
      ...(validNodeId && { node_id: validNodeId }),
      ...(workingDir.trim() && { working_dir: workingDir.trim() }),
      ...(validProjectId && { project_id: validProjectId }),
    })
  }

  return (
    <Sheet open={open} onOpenChange={setOpen}>
      {children && <SheetTrigger render={children} />}
      <SheetContent side={isMobile ? "bottom" : "right"} className={isMobile ? "rounded-t-xl" : ""}>
        <SheetHeader>
          <SheetTitle>New Session</SheetTitle>
          <SheetDescription>Create a new agent session.</SheetDescription>
        </SheetHeader>
        <form onSubmit={handleSubmit} className="flex flex-1 flex-col gap-4 overflow-y-auto px-4">
          <div className="space-y-1.5">
            <label htmlFor="prompt" className="text-sm font-medium">
              Prompt
            </label>
            <Textarea
              id="prompt"
              value={prompt}
              onChange={(e) => setPrompt(e.target.value)}
              placeholder="Describe the task for the agent..."
              rows={3}
              required
            />
          </div>

          {workspaces.length > 0 && (
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
              <div className="space-y-1.5">
                <label htmlFor="workspace" className="text-sm font-medium">
                  Workspace <span className="text-muted-foreground font-normal">(optional)</span>
                </label>
                <Select
                  value={workspaceId}
                  onValueChange={(v) => {
                    setWorkspaceId(v ?? "")
                    setProjectId("")
                  }}
                >
                  <SelectTrigger id="workspace" className="w-full">
                    <SelectValue placeholder="Personal" />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="">Personal</SelectItem>
                    {workspaces.map((w) => (
                      <SelectItem key={w.id} value={w.id}>
                        {w.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
              {workspaceId && (
                <div className="space-y-1.5">
                  <label htmlFor="project" className="text-sm font-medium">
                    Project{" "}
                    <span className="text-muted-foreground font-normal">
                      (optional)
                    </span>
                  </label>
                  <Select
                    value={projectId}
                    onValueChange={(v) => setProjectId(v ?? "")}
                  >
                    <SelectTrigger id="project" className="w-full">
                      <SelectValue placeholder="No project" />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="">No project</SelectItem>
                      {visibleProjects.map((p) => (
                        <SelectItem key={p.id} value={p.id}>
                          {p.repo_owner}/{p.repo_name}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
              )}
            </div>
          )}

          {onlineNodes.length > 0 && (
            <div className="space-y-1.5">
              <label htmlFor="node" className="text-sm font-medium">
                Node <span className="text-muted-foreground font-normal">(optional)</span>
              </label>
              <Select value={nodeId} onValueChange={(v) => setNodeId(v ?? "")}>
                <SelectTrigger id="node" className="w-full">
                  <SelectValue placeholder="Auto-assign" />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="">Auto-assign</SelectItem>
                  {onlineNodes.map((node) => (
                    <SelectItem key={node.id} value={node.id}>
                      {node.name} ({node.active_sessions}/{node.max_sessions})
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          )}

          <div className="space-y-1.5">
            <label htmlFor="workingDir" className="text-sm font-medium">
              Working directory <span className="text-muted-foreground font-normal">(optional)</span>
            </label>
            <Input
              id="workingDir"
              value={workingDir}
              onChange={(e) => setWorkingDir(e.target.value)}
              placeholder="/path/to/project"
            />
          </div>

          {mutation.isError && (
            <p className="text-sm text-destructive">
              {mutation.error instanceof Error ? mutation.error.message : "Failed to create session"}
            </p>
          )}

          <SheetFooter>
            <Button
              type="submit"
              disabled={!prompt.trim() || mutation.isPending}
              className="w-full"
              size="lg"
            >
              {mutation.isPending ? "Creating..." : "Create Session"}
            </Button>
          </SheetFooter>
        </form>
      </SheetContent>
    </Sheet>
  )
}
