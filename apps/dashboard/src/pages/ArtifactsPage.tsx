import { useQuery } from "@tanstack/react-query"
import { api } from "@/lib/api"
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
import { Plus } from "lucide-react"
import { Link } from "react-router-dom"
import { CreateArtifactSheet } from "@/components/factory/CreateArtifactSheet"

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
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-xl font-bold tracking-tight md:text-2xl">Artifacts</h1>
          <p className="text-sm text-muted-foreground">
            Production artifacts managed by Forge.
          </p>
        </div>
        <CreateArtifactSheet>
          <Button size="sm">
            <Plus className="mr-1.5 h-4 w-4" />
            Register Artifact
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
                  <Plus className="mr-1.5 h-4 w-4" />
                  Register your first artifact
                </Button>
              </CreateArtifactSheet>
            </div>
          ) : (
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
                      <Link to={`/artifacts/${a.id}`} className="font-medium hover:underline">
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
