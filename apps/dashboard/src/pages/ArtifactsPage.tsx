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

const STATE_VARIANT: Record<string, "default" | "secondary" | "destructive" | "outline"> = {
  draft: "outline",
  in_progress: "default",
  under_review: "secondary",
  released: "default",
  archived: "secondary",
}

export function ArtifactsPage() {
  const { data, isLoading } = useQuery({
    queryKey: ["artifacts"],
    queryFn: api.getArtifacts,
    refetchInterval: 5000,
  })

  const artifacts = data?.artifacts ?? []

  return (
    <div className="space-y-4 md:space-y-6">
      <div>
        <h1 className="text-xl font-bold tracking-tight md:text-2xl">Artifacts</h1>
        <p className="text-sm text-muted-foreground">
          Production artifacts managed by Forge.
        </p>
      </div>

      <Card>
        <CardHeader className="px-4 md:px-6">
          <CardTitle className="text-base md:text-lg">
            All Artifacts {artifacts.length > 0 && <span className="text-muted-foreground font-normal">({artifacts.length})</span>}
          </CardTitle>
        </CardHeader>
        <CardContent className="px-4 md:px-6">
          {isLoading ? (
            <p className="py-8 text-center text-muted-foreground">Loading...</p>
          ) : artifacts.length === 0 ? (
            <p className="py-8 text-center text-muted-foreground">
              No artifacts yet. Artifacts appear when Forge processes work through the pipeline.
            </p>
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>ID</TableHead>
                  <TableHead>Kind</TableHead>
                  <TableHead>State</TableHead>
                  <TableHead>Owner</TableHead>
                  <TableHead>Version</TableHead>
                  <TableHead>Updated</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {artifacts.map((a) => (
                  <TableRow key={a.id}>
                    <TableCell className="font-mono text-sm">{a.id}</TableCell>
                    <TableCell>
                      <Badge variant="outline">{a.kind}</Badge>
                    </TableCell>
                    <TableCell>
                      <Badge variant={STATE_VARIANT[a.state] || "secondary"}>
                        {a.state}
                      </Badge>
                    </TableCell>
                    <TableCell className="text-muted-foreground">{a.owner}</TableCell>
                    <TableCell className="font-mono">v{a.current_version}</TableCell>
                    <TableCell className="text-xs text-muted-foreground">
                      {new Date(a.updated_at).toLocaleString()}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          )}
        </CardContent>
      </Card>
    </div>
  )
}
