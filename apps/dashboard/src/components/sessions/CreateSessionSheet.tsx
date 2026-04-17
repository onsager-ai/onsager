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
