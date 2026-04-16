import { useParams, Link } from "react-router-dom"
import { useQuery } from "@tanstack/react-query"
import { api } from "@/lib/api"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { Badge } from "@/components/ui/badge"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { ArrowLeft } from "lucide-react"

const STATE_VARIANT: Record<string, "default" | "secondary" | "destructive" | "outline"> = {
  draft: "outline",
  in_progress: "default",
  under_review: "secondary",
  released: "default",
  archived: "secondary",
}

export function ArtifactDetailPage() {
  const { id } = useParams<{ id: string }>()

  const { data, isLoading, error } = useQuery({
    queryKey: ["artifact", id],
    queryFn: () => api.getArtifact(id!),
    enabled: !!id,
    refetchInterval: 5000,
  })

  const artifact = data?.artifact

  if (isLoading) {
    return (
      <div className="flex min-h-[200px] items-center justify-center">
        <p className="text-muted-foreground">Loading...</p>
      </div>
    )
  }

  if (error || !artifact) {
    return (
      <div className="space-y-4">
        <Link to="/artifacts" className="inline-flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground">
          <ArrowLeft className="h-4 w-4" /> Back to Artifacts
        </Link>
        <p className="text-destructive">Artifact not found.</p>
      </div>
    )
  }

  return (
    <div className="space-y-4 md:space-y-6">
      <Link to="/artifacts" className="inline-flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground">
        <ArrowLeft className="h-4 w-4" /> Back to Artifacts
      </Link>

      <div className="flex items-start justify-between gap-4">
        <div>
          <h1 className="text-xl font-bold tracking-tight md:text-2xl">{artifact.name}</h1>
          <p className="text-sm text-muted-foreground font-mono">{artifact.id}</p>
        </div>
        <Badge variant={STATE_VARIANT[artifact.state] || "secondary"} className="text-sm">
          {artifact.state.replace("_", " ")}
        </Badge>
      </div>

      {/* Metadata */}
      <div className="grid grid-cols-2 gap-3 md:grid-cols-4">
        <Card>
          <CardContent className="px-3 py-3">
            <div className="text-xs text-muted-foreground">Kind</div>
            <div className="font-medium">{artifact.kind}</div>
          </CardContent>
        </Card>
        <Card>
          <CardContent className="px-3 py-3">
            <div className="text-xs text-muted-foreground">Owner</div>
            <div className="font-medium">{artifact.owner}</div>
          </CardContent>
        </Card>
        <Card>
          <CardContent className="px-3 py-3">
            <div className="text-xs text-muted-foreground">Version</div>
            <div className="font-mono font-medium">v{artifact.current_version}</div>
          </CardContent>
        </Card>
        <Card>
          <CardContent className="px-3 py-3">
            <div className="text-xs text-muted-foreground">Created</div>
            <div className="text-sm">{new Date(artifact.created_at).toLocaleDateString()}</div>
          </CardContent>
        </Card>
      </div>

      {/* Version History */}
      <Card>
        <CardHeader className="px-4 md:px-6">
          <CardTitle className="text-base md:text-lg">
            Version History
            {artifact.versions && artifact.versions.length > 0 && (
              <span className="ml-2 text-muted-foreground font-normal">
                ({artifact.versions.length})
              </span>
            )}
          </CardTitle>
        </CardHeader>
        <CardContent className="px-4 md:px-6">
          {!artifact.versions || artifact.versions.length === 0 ? (
            <p className="py-4 text-center text-sm text-muted-foreground">
              No versions yet. Versions are created as Forge shapes this artifact.
            </p>
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Version</TableHead>
                  <TableHead>Summary</TableHead>
                  <TableHead>Session</TableHead>
                  <TableHead>Created</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {artifact.versions
                  .sort((a, b) => b.version - a.version)
                  .map((v) => (
                    <TableRow key={v.version}>
                      <TableCell className="font-mono">v{v.version}</TableCell>
                      <TableCell className="max-w-[300px] truncate">
                        {v.change_summary || "-"}
                      </TableCell>
                      <TableCell>
                        <Link
                          to={`/sessions/${v.created_by_session}`}
                          className="font-mono text-xs hover:underline"
                        >
                          {v.created_by_session.slice(0, 8)}
                        </Link>
                      </TableCell>
                      <TableCell className="text-xs text-muted-foreground">
                        {new Date(v.created_at).toLocaleString()}
                      </TableCell>
                    </TableRow>
                  ))}
              </TableBody>
            </Table>
          )}
        </CardContent>
      </Card>

      {/* Lineage */}
      {artifact.vertical_lineage && artifact.vertical_lineage.length > 0 && (
        <Card>
          <CardHeader className="px-4 md:px-6">
            <CardTitle className="text-base md:text-lg">Vertical Lineage</CardTitle>
          </CardHeader>
          <CardContent className="px-4 md:px-6">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Version</TableHead>
                  <TableHead>Session</TableHead>
                  <TableHead>Recorded</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {artifact.vertical_lineage.map((entry, i) => (
                  <TableRow key={i}>
                    <TableCell className="font-mono">v{entry.version}</TableCell>
                    <TableCell>
                      <Link
                        to={`/sessions/${entry.session_id}`}
                        className="font-mono text-xs hover:underline"
                      >
                        {entry.session_id.slice(0, 8)}
                      </Link>
                    </TableCell>
                    <TableCell className="text-xs text-muted-foreground">
                      {new Date(entry.recorded_at).toLocaleString()}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </CardContent>
        </Card>
      )}
    </div>
  )
}
