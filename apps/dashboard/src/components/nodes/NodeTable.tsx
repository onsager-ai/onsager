import type { Node } from "@/lib/api"
import { NodeStatusBadge } from "./NodeStatusBadge"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { formatDistanceToNow } from "@/lib/utils"

interface NodeTableProps {
  nodes: Node[]
}

function NodeCard({ node }: { node: Node }) {
  return (
    <div className="rounded-lg border p-3 space-y-2">
      <div className="flex items-center justify-between">
        <span className="font-medium">{node.name}</span>
        <NodeStatusBadge status={node.status} />
      </div>
      <div className="grid grid-cols-2 gap-x-4 gap-y-1 text-sm">
        <div>
          <span className="text-muted-foreground">Host</span>
          <p className="truncate">{node.hostname}</p>
        </div>
        <div>
          <span className="text-muted-foreground">Sessions</span>
          <p className="font-mono">{node.active_sessions}/{node.max_sessions}</p>
        </div>
        <div>
          <span className="text-muted-foreground">Heartbeat</span>
          <p className="text-xs">{formatDistanceToNow(node.last_heartbeat)}</p>
        </div>
        <div>
          <span className="text-muted-foreground">Registered</span>
          <p className="text-xs">{formatDistanceToNow(node.registered_at)}</p>
        </div>
      </div>
    </div>
  )
}

export function NodeTable({ nodes }: NodeTableProps) {
  if (nodes.length === 0) {
    return (
      <p className="py-8 text-center text-muted-foreground">No nodes registered</p>
    )
  }

  return (
    <>
      {/* Mobile: card list */}
      <div className="flex flex-col gap-2 md:hidden">
        {nodes.map((node) => (
          <NodeCard key={node.id} node={node} />
        ))}
      </div>

      {/* Desktop: table */}
      <div className="hidden md:block">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Name</TableHead>
              <TableHead>Hostname</TableHead>
              <TableHead>Status</TableHead>
              <TableHead>Sessions</TableHead>
              <TableHead>Last Heartbeat</TableHead>
              <TableHead>Registered</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {nodes.map((node) => (
              <TableRow key={node.id}>
                <TableCell className="font-medium">{node.name}</TableCell>
                <TableCell className="text-muted-foreground">{node.hostname}</TableCell>
                <TableCell>
                  <NodeStatusBadge status={node.status} />
                </TableCell>
                <TableCell>
                  <span className="font-mono text-sm">
                    {node.active_sessions}/{node.max_sessions}
                  </span>
                </TableCell>
                <TableCell className="text-sm text-muted-foreground">
                  {formatDistanceToNow(node.last_heartbeat)}
                </TableCell>
                <TableCell className="text-sm text-muted-foreground">
                  {formatDistanceToNow(node.registered_at)}
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </div>
    </>
  )
}
