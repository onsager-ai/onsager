import { type FormEvent, useState, type ReactElement } from "react"
import { useMutation, useQueryClient } from "@tanstack/react-query"
import { api, type RegisterArtifactRequest } from "@/lib/api"
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

const ARTIFACT_KINDS = [
  { value: "code", label: "Code" },
  { value: "document", label: "Document" },
  { value: "report", label: "Report" },
  { value: "dataset", label: "Dataset" },
  { value: "config", label: "Config" },
  { value: "api_call", label: "API Call" },
]

interface CreateArtifactSheetProps {
  children: ReactElement
}

export function CreateArtifactSheet({ children }: CreateArtifactSheetProps) {
  const [open, setOpen] = useState(false)
  const [name, setName] = useState("")
  const [kind, setKind] = useState("code")
  const [owner, setOwner] = useState("")
  const [description, setDescription] = useState("")
  const [workingDir, setWorkingDir] = useState("")
  const isMobile = useIsMobile()
  const queryClient = useQueryClient()

  const mutation = useMutation({
    mutationFn: (req: RegisterArtifactRequest) => api.registerArtifact(req),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["artifacts"] })
      setOpen(false)
      setName("")
      setKind("code")
      setOwner("")
      setDescription("")
      setWorkingDir("")
    },
  })

  function handleSubmit(e: FormEvent) {
    e.preventDefault()
    if (!name.trim() || !owner.trim()) return
    mutation.mutate({
      name: name.trim(),
      kind,
      owner: owner.trim(),
      ...(description.trim() && { description: description.trim() }),
      ...(workingDir.trim() && { working_dir: workingDir.trim() }),
    })
  }

  return (
    <Sheet open={open} onOpenChange={setOpen}>
      <SheetTrigger render={children} />
      <SheetContent side={isMobile ? "bottom" : "right"} className={isMobile ? "rounded-t-xl" : ""}>
        <SheetHeader>
          <SheetTitle>Register Artifact</SheetTitle>
          <SheetDescription>
            Register a new artifact in the factory pipeline. Forge will pick it up and begin shaping.
          </SheetDescription>
        </SheetHeader>
        <form onSubmit={handleSubmit} className="flex flex-1 flex-col gap-4 overflow-y-auto px-4">
          <div className="space-y-1.5">
            <label htmlFor="artifact-name" className="text-sm font-medium">
              Name
            </label>
            <Input
              id="artifact-name"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="my-service"
              required
            />
          </div>

          <div className="space-y-1.5">
            <label htmlFor="artifact-kind" className="text-sm font-medium">
              Kind
            </label>
            <select
              id="artifact-kind"
              value={kind}
              onChange={(e) => setKind(e.target.value)}
              className="h-8 w-full rounded-lg border border-input bg-transparent px-2.5 text-base outline-none focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50 md:text-sm dark:bg-input/30"
            >
              {ARTIFACT_KINDS.map((k) => (
                <option key={k.value} value={k.value}>
                  {k.label}
                </option>
              ))}
            </select>
          </div>

          <div className="space-y-1.5">
            <label htmlFor="artifact-owner" className="text-sm font-medium">
              Owner
            </label>
            <Input
              id="artifact-owner"
              value={owner}
              onChange={(e) => setOwner(e.target.value)}
              placeholder="team-name or username"
              required
            />
          </div>

          <div className="space-y-1.5">
            <label htmlFor="artifact-description" className="text-sm font-medium">
              Description <span className="text-muted-foreground font-normal">(optional)</span>
            </label>
            <textarea
              id="artifact-description"
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              placeholder="What should this artifact accomplish..."
              rows={3}
              className="w-full min-w-0 rounded-lg border border-input bg-transparent px-2.5 py-2 text-base transition-colors outline-none placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50 md:text-sm dark:bg-input/30"
            />
          </div>

          <div className="space-y-1.5">
            <label htmlFor="artifact-workdir" className="text-sm font-medium">
              Working directory <span className="text-muted-foreground font-normal">(optional)</span>
            </label>
            <Input
              id="artifact-workdir"
              value={workingDir}
              onChange={(e) => setWorkingDir(e.target.value)}
              placeholder="/path/to/project"
            />
          </div>

          {mutation.isError && (
            <p className="text-sm text-destructive">
              {mutation.error instanceof Error ? mutation.error.message : "Failed to register artifact"}
            </p>
          )}

          <SheetFooter>
            <Button
              type="submit"
              disabled={!name.trim() || !owner.trim() || mutation.isPending}
              className="w-full"
              size="lg"
            >
              {mutation.isPending ? "Registering..." : "Register Artifact"}
            </Button>
          </SheetFooter>
        </form>
      </SheetContent>
    </Sheet>
  )
}
