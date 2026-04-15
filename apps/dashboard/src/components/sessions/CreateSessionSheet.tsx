import { type FormEvent, useState, type ReactElement } from "react"
import { useMutation, useQueryClient } from "@tanstack/react-query"
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

interface CreateSessionSheetProps {
  children: ReactElement
}

export function CreateSessionSheet({ children }: CreateSessionSheetProps) {
  const [open, setOpen] = useState(false)
  const [prompt, setPrompt] = useState("")
  const [nodeId, setNodeId] = useState("")
  const [workingDir, setWorkingDir] = useState("")
  const isMobile = useIsMobile()
  const queryClient = useQueryClient()
  const { data: nodesData } = useNodes()
  const onlineNodes = nodesData?.nodes.filter((n) => n.status === "online") ?? []

  const mutation = useMutation({
    mutationFn: (task: TaskRequest) => api.createTask(task),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["sessions"] })
      setOpen(false)
      setPrompt("")
      setNodeId("")
      setWorkingDir("")
    },
  })

  function handleSubmit(e: FormEvent) {
    e.preventDefault()
    if (!prompt.trim()) return
    const validNodeId = onlineNodes.some((n) => n.id === nodeId) ? nodeId : ""
    mutation.mutate({
      prompt: prompt.trim(),
      ...(validNodeId && { node_id: validNodeId }),
      ...(workingDir.trim() && { working_dir: workingDir.trim() }),
    })
  }

  return (
    <Sheet open={open} onOpenChange={setOpen}>
      <SheetTrigger render={children} />
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
            <textarea
              id="prompt"
              value={prompt}
              onChange={(e) => setPrompt(e.target.value)}
              placeholder="Describe the task for the agent..."
              rows={3}
              required
              className="w-full min-w-0 rounded-lg border border-input bg-transparent px-2.5 py-2 text-base transition-colors outline-none placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50 md:text-sm dark:bg-input/30"
            />
          </div>

          {onlineNodes.length > 0 && (
            <div className="space-y-1.5">
              <label htmlFor="node" className="text-sm font-medium">
                Node <span className="text-muted-foreground font-normal">(optional)</span>
              </label>
              <select
                id="node"
                value={nodeId}
                onChange={(e) => setNodeId(e.target.value)}
                className="h-8 w-full rounded-lg border border-input bg-transparent px-2.5 text-base outline-none focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50 md:text-sm dark:bg-input/30"
              >
                <option value="">Auto-assign</option>
                {onlineNodes.map((node) => (
                  <option key={node.id} value={node.id}>
                    {node.name} ({node.active_sessions}/{node.max_sessions})
                  </option>
                ))}
              </select>
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
