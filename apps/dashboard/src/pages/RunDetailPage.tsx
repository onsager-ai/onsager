import { Link, useParams } from 'react-router-dom'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { ArrowLeft } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { operateApi, runsApi } from '@/lib/api'
import { SessionFeed } from '@/components/runs/SessionFeed'
import { Button } from '@/components/ui/button'

const STATUS_VARIANT: Record<string, 'default' | 'secondary' | 'destructive'> = {
  completed: 'default',
  running: 'secondary',
  failed: 'destructive',
  aborted: 'destructive',
}

export function RunDetailPage() {
  const { id } = useParams<{ id: string }>()
  const qc = useQueryClient()
  const { data } = useQuery({
    queryKey: ['run', id],
    queryFn: () => runsApi.get(id!),
    enabled: !!id,
    refetchInterval: (q) => (q.state.data?.run.status === 'running' ? 2000 : false),
  })

  const abort = useMutation({
    mutationFn: () => operateApi.abortRun(id!),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['run', id] }),
  })

  if (!data) return <p className="text-sm text-muted-foreground">Loading...</p>
  const { run, timeline, artifacts, sessions } = data

  return (
    <>
      <Link
        to={`/workflows/${run.workflow_id}`}
        className="inline-flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
      >
        <ArrowLeft className="h-4 w-4" /> Workflow
      </Link>
      <div className="flex items-center justify-between">
        <h1 className="font-mono text-lg font-bold tracking-tight">{run.id}</h1>
        <div className="flex items-center gap-2">
          {run.status === 'running' && (
            <Button
              variant="destructive"
              size="sm"
              disabled={abort.isPending}
              onClick={() => abort.mutate()}
            >
              Abort
            </Button>
          )}
          <Badge variant={STATUS_VARIANT[run.status] ?? 'secondary'}>{run.status}</Badge>
        </div>
      </div>
      {run.error && (
        <div className="rounded-md border border-red-300 bg-red-50 px-3 py-2 text-sm text-red-900 dark:border-red-900 dark:bg-red-950/40 dark:text-red-200">
          {run.error}
        </div>
      )}

      <Card>
        <CardHeader>
          <CardTitle className="text-base">Timeline</CardTitle>
        </CardHeader>
        <CardContent className="space-y-2">
          {timeline.map((event) => (
            <div key={event.id} className="flex items-baseline gap-3 text-sm">
              <span className="w-40 shrink-0 text-xs text-muted-foreground">
                {new Date(event.created_at).toLocaleTimeString()}
              </span>
              <Badge variant="outline">{event.kind}</Badge>
              <span className="truncate text-xs text-muted-foreground">
                {summarize(event.kind, event.payload)}
              </span>
            </div>
          ))}
        </CardContent>
      </Card>

      {sessions.map((s) => (
        <SessionFeed key={s.id} sessionId={s.id} status={s.status} />
      ))}

      <Card>
        <CardHeader>
          <CardTitle className="text-base">Artifacts</CardTitle>
        </CardHeader>
        <CardContent className="space-y-2">
          {artifacts.length === 0 && (
            <p className="text-sm text-muted-foreground">
              {run.status === 'running' ? 'Nothing emitted yet.' : 'No artifacts.'}
            </p>
          )}
          {artifacts.map((a) => (
            <Link
              key={a.id}
              to={`/artifacts/${a.id}`}
              className="flex items-center gap-3 rounded-md border p-3 text-sm transition-colors hover:bg-muted/40"
            >
              <Badge variant="outline">{a.kind}</Badge>
              <span className="font-medium">{a.name}</span>
              <span className="ml-auto text-xs text-muted-foreground">
                {a.provenance.kind} / {a.provenance.source}
              </span>
            </Link>
          ))}
        </CardContent>
      </Card>
    </>
  )
}

function summarize(kind: string, payload: Record<string, unknown>): string {
  switch (kind) {
    case 'run.started':
      return String(payload.workflow_name ?? '')
    case 'session.started':
    case 'session.completed':
    case 'session.failed':
      return String(payload.session_id ?? '').slice(0, 18)
    case 'artifact.emitted':
      return String(payload.name ?? payload.artifact_id ?? '')
    case 'run.failed':
      return String(payload.error ?? '')
    default:
      return ''
  }
}
