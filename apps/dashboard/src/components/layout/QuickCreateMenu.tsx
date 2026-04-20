import { useState } from "react"
import { Building2, Package, Plus, Terminal } from "lucide-react"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { CreateSessionSheet } from "@/components/sessions/CreateSessionSheet"
import { CreateArtifactSheet } from "@/components/factory/CreateArtifactSheet"
import { NewWorkspaceDialog } from "@/components/workspaces/NewWorkspaceDialog"
import { useAuth } from "@/lib/auth"

export function QuickCreateMenu() {
  const { user, authEnabled } = useAuth()
  const canCreateWorkspace = authEnabled && !!user
  const [sessionOpen, setSessionOpen] = useState(false)
  const [artifactOpen, setArtifactOpen] = useState(false)
  const [workspaceOpen, setWorkspaceOpen] = useState(false)

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
          {canCreateWorkspace && (
            <>
              <DropdownMenuItem onClick={() => setWorkspaceOpen(true)}>
                <Building2 className="mr-2 h-4 w-4" />
                New Workspace
              </DropdownMenuItem>
              <DropdownMenuSeparator />
            </>
          )}
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
      {canCreateWorkspace && (
        <NewWorkspaceDialog
          open={workspaceOpen}
          onOpenChange={setWorkspaceOpen}
        />
      )}
    </>
  )
}
