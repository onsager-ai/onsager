import { Link } from "react-router-dom"
import type { Session } from "@/lib/api"
import { SessionStateBadge } from "./SessionStateBadge"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { formatDistanceToNow } from "@/lib/utils"
import { ChevronRight, Plus } from "lucide-react"
import { CreateSessionSheet } from "./CreateSessionSheet"
import { Button } from "@/components/ui/button"

interface SessionTableProps {
  sessions: Session[]
  workspaceSlug: string
}

function SessionCard({ session, workspaceSlug }: { session: Session; workspaceSlug: string }) {
  return (
    <Link
      to={`/workspaces/${workspaceSlug}/sessions/${session.id}`}
      className="flex items-center gap-3 rounded-lg border p-3 transition-colors active:bg-accent"
    >
      <div className="min-w-0 flex-1 space-y-1">
        <div className="flex items-center gap-2">
          <span className="font-mono text-sm font-medium text-blue-500">
            {session.id.slice(0, 8)}
          </span>
          <SessionStateBadge state={session.state} />
        </div>
        <p className="truncate text-sm text-muted-foreground">
          {session.prompt.slice(0, 80)}
        </p>
        <p className="text-xs text-muted-foreground">
          {formatDistanceToNow(session.created_at)}
        </p>
      </div>
      <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground" />
    </Link>
  )
}

export function SessionTable({ sessions, workspaceSlug }: SessionTableProps) {
  if (sessions.length === 0) {
    return (
      <div className="flex flex-col items-center gap-3 py-8">
        <p className="text-center text-muted-foreground">No sessions yet</p>
        <CreateSessionSheet>
          <Button size="sm" variant="outline">
            <Plus className="h-4 w-4" data-icon="inline-start" />
            New Session
          </Button>
        </CreateSessionSheet>
      </div>
    )
  }

  return (
    <>
      {/* Mobile: card list */}
      <div className="flex flex-col gap-2 md:hidden">
        {sessions.map((session) => (
          <SessionCard
            key={session.id}
            session={session}
            workspaceSlug={workspaceSlug}
          />
        ))}
      </div>

      {/* Desktop: table */}
      <div className="hidden md:block">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>ID</TableHead>
              <TableHead>Node</TableHead>
              <TableHead>State</TableHead>
              <TableHead>Prompt</TableHead>
              <TableHead>Created</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {sessions.map((session) => (
              <TableRow key={session.id}>
                <TableCell>
                  <Link
                    to={`/workspaces/${workspaceSlug}/sessions/${session.id}`}
                    className="font-mono text-sm text-blue-500 hover:underline"
                  >
                    {session.id.slice(0, 8)}
                  </Link>
                </TableCell>
                <TableCell className="text-muted-foreground">{session.node_id.slice(0, 8)}</TableCell>
                <TableCell>
                  <SessionStateBadge state={session.state} />
                </TableCell>
                <TableCell className="max-w-[300px] truncate text-sm">
                  {session.prompt.slice(0, 80)}
                </TableCell>
                <TableCell className="text-sm text-muted-foreground">
                  {formatDistanceToNow(session.created_at)}
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </div>
    </>
  )
}
