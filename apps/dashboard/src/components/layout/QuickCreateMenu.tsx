import { useState } from "react"
import { Package, Plus, Terminal } from "lucide-react"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { CreateSessionSheet } from "@/components/sessions/CreateSessionSheet"
import { CreateArtifactSheet } from "@/components/factory/CreateArtifactSheet"

export function QuickCreateMenu() {
  const [sessionOpen, setSessionOpen] = useState(false)
  const [artifactOpen, setArtifactOpen] = useState(false)

  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger
          render={
            <Button
              variant="ghost"
              size="icon"
              className="h-9 w-9"
              aria-label="Create"
            />
          }
        >
          <Plus className="h-5 w-5" />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="w-48">
          <DropdownMenuItem onClick={() => setSessionOpen(true)}>
            <Terminal className="mr-2 h-4 w-4" />
            New Session
          </DropdownMenuItem>
          <DropdownMenuItem onClick={() => setArtifactOpen(true)}>
            <Package className="mr-2 h-4 w-4" />
            Register Artifact
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>

      {sessionOpen && (
        <CreateSessionSheet open={sessionOpen} onOpenChange={setSessionOpen} />
      )}
      {artifactOpen && (
        <CreateArtifactSheet open={artifactOpen} onOpenChange={setArtifactOpen} />
      )}
    </>
  )
}
