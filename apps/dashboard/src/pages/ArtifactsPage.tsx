import { useQuery } from "@tanstack/react-query"
import { api, type SpineArtifact } from "@/lib/api"
import { useActiveWorkspace } from "@/lib/workspace"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { ChevronRight, Plus } from "lucide-react"
import { Link } from "react-router-dom"
import { CreateArtifactSheet } from "@/components/factory/CreateArtifactSheet"
import { usePageHeader } from "@/components/layout/PageHeader"

const STATE_VARIANT: Record<string, "default" | "secondary" | "destructive" | "outline"> = {
  draft: "outline",
  in_progress: "default",
  under_review: "secondary",
  released: "default",
  archived: "secondary",
}

export function ArtifactsPage() {
  usePageHeader({ title: "Artifacts" })
  const workspace = useActiveWorkspace()
  const { data, isLoading } = useQuery({
    queryKey: ["artifacts", workspace.id],
    queryFn: () => api.getArtifacts(workspace.id),
    refetchInterval: 5000,
  })

  const artifacts = data?.artifacts ?? []

  return (
    <div className="space-y-4 md:space-y-6">
      <div className="flex items-center justify-between gap-4">
        <div>
          <h1 className="hidden text-2xl font-bold tracking-tight md:block">Artifacts</h1>
          <p className="text-sm text-muted-foreground">
            Production artifacts managed by Forge.
          </p>
        </div>
        <CreateArtifactSheet>
          <Button size="sm" className="shrink-0">
            <Plus className="h-4 w-4" data-icon="inline-start" />
            <span className="hidden sm:inline">Register Artifact</span>
            <span className="sm:hidden">New</span>
          </Button>
        </CreateArtifactSheet>
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
            <div className="py-8 text-center">
              <p className="text-muted-foreground">
                No artifacts yet. Register one to start the factory pipeline.
              </p>
              <CreateArtifactSheet>
                <Button variant="outline" className="mt-4">
                  <Plus className="h-4 w-4" data-icon="inline-start" />
                  Register your first artifact
                </Button>
              </CreateArtifactSheet>
            </div>
          ) : (
            <>
              {/* Mobile: card list */}
              <div className="flex flex-col gap-2 md:hidden">
                {artifacts.map((a) => (
                  <ArtifactCard key={a.id} artifact={a} workspaceSlug={workspace.slug} />
                ))}
              </div>

              {/* Desktop: table */}
              <div className="hidden md:block">
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead>Name</TableHead>
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
                        <TableCell>
                          <Link to={`/workspaces/${workspace.slug}/artifacts/${a.id}`} className="font-medium hover:underline">
                            {a.name ?? a.id}
                          </Link>
                        </TableCell>
                        <TableCell>
                          <Badge variant="outline">{a.kind}</Badge>
                        </TableCell>
                        <TableCell>
                          <Badge variant={STATE_VARIANT[a.state] || "secondary"}>
                            {a.state.replace("_", " ")}
                          </Badge>
                        </TableCell>
                        <TableCell className="text-muted-foreground">
                          {a.owner ?? <span className="italic opacity-60">—</span>}
                        </TableCell>
                        <TableCell className="font-mono">v{a.current_version}</TableCell>
                        <TableCell className="text-xs text-muted-foreground">
                          {new Date(a.updated_at).toLocaleString()}
                        </TableCell>
                      </TableRow>
                    ))}
                  </TableBody>
                </Table>
              </div>
            </>
          )}
        </CardContent>
      </Card>
    </div>
  )
}

function ArtifactCard({ artifact, workspaceSlug }: { artifact: SpineArtifact; workspaceSlug: string }) {
  return (
    <Link
      to={`/workspaces/${workspaceSlug}/artifacts/${artifact.id}`}
      className="flex items-center gap-3 rounded-lg border p-3 transition-colors active:bg-accent"
    >
      <div className="min-w-0 flex-1 space-y-1.5">
        <div className="flex items-center gap-2">
          <span className="truncate text-sm font-medium">{artifact.name ?? artifact.id}</span>
          <Badge variant={STATE_VARIANT[artifact.state] || "secondary"} className="shrink-0">
            {artifact.state.replace("_", " ")}
          </Badge>
        </div>
        <div className="flex items-center gap-2 text-xs text-muted-foreground">
          <Badge variant="outline" className="shrink-0">{artifact.kind}</Badge>
          <span className="truncate">{artifact.owner ?? "—"}</span>
          <span className="shrink-0 font-mono">v{artifact.current_version}</span>
        </div>
      </div>
      <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground" />
    </Link>
  )
}
