import { Link, useParams } from 'react-router-dom'
import { useQuery } from '@tanstack/react-query'
import { ArrowLeft } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { artifactsApi, decodeInlineContent } from '@/lib/api'
import { useWorkspace } from '@/lib/workspace'

export function ArtifactsPage() {
  const ws = useWorkspace()
  const { data, isLoading } = useQuery({
    queryKey: ['artifacts', ws.id],
    queryFn: () => artifactsApi.list(ws.id),
  })
  return (
    <>
      <h1 className="text-xl font-bold tracking-tight">Artifacts</h1>
      {isLoading && <p className="text-sm text-muted-foreground">Loading...</p>}
      {data?.artifacts.map((a) => (
        <Link key={a.id} to={`/artifacts/${a.id}`} className="block">
          <Card className="transition-colors hover:bg-muted/40">
            <CardHeader>
              <div className="flex items-center gap-3">
                <Badge variant="outline">{a.kind}</Badge>
                <CardTitle className="text-base">{a.name}</CardTitle>
                <span className="ml-auto text-xs text-muted-foreground">
                  {a.provenance.kind} / {a.provenance.source} ·{' '}
                  {new Date(a.created_at).toLocaleString()}
                </span>
              </div>
            </CardHeader>
          </Card>
        </Link>
      ))}
      {data && data.artifacts.length === 0 && (
        <p className="text-sm text-muted-foreground">
          No artifacts yet — they appear when runs produce output.
        </p>
      )}
    </>
  )
}

export function ArtifactDetailPage() {
  const { id } = useParams<{ id: string }>()
  const { data } = useQuery({
    queryKey: ['artifact', id],
    queryFn: () => artifactsApi.get(id!),
    enabled: !!id,
  })
  if (!data) return <p className="text-sm text-muted-foreground">Loading...</p>
  const a = data.artifact
  const content = decodeInlineContent(a.content_uri)
  return (
    <>
      <Link
        to="/artifacts"
        className="inline-flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
      >
        <ArrowLeft className="h-4 w-4" /> Artifacts
      </Link>
      <div className="flex items-center gap-3">
        <h1 className="text-xl font-bold tracking-tight">{a.name}</h1>
        <Badge variant="outline">{a.kind}</Badge>
        <Badge variant="secondary">
          {a.provenance.kind} / {a.provenance.source}
        </Badge>
      </div>
      <p className="font-mono text-xs text-muted-foreground">{a.id}</p>
      {a.run_id && (
        <p className="text-sm text-muted-foreground">
          Produced by{' '}
          <Link to={`/runs/${a.run_id}`} className="font-mono hover:underline">
            {a.run_id}
          </Link>
        </p>
      )}
      <Card>
        <CardHeader>
          <CardTitle className="text-base">Content</CardTitle>
        </CardHeader>
        <CardContent>
          {content !== null ? (
            <pre className="overflow-x-auto whitespace-pre-wrap rounded-md bg-muted/40 p-3 text-sm">
              {content}
            </pre>
          ) : (
            <p className="text-sm text-muted-foreground">
              No inline content (external reference).
            </p>
          )}
        </CardContent>
      </Card>
    </>
  )
}
