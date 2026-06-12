import { Link } from 'react-router-dom'
import { useQuery } from '@tanstack/react-query'
import { Badge } from '@/components/ui/badge'
import { operateApi } from '@/lib/api'
import { useWorkspace } from '@/lib/workspace'

/** The workspace audit feed (polls; rows deep-link to their run). */
export function ActivityPage() {
  const ws = useWorkspace()
  const { data, isLoading } = useQuery({
    queryKey: ['activity', ws.id],
    queryFn: () => operateApi.activity(ws.id),
    refetchInterval: 3000,
  })
  return (
    <>
      <h1 className="text-xl font-bold tracking-tight">Activity</h1>
      {isLoading && <p className="text-sm text-muted-foreground">Loading...</p>}
      <div className="space-y-1">
        {data?.events.map((e) => {
          const row = (
            <div className="flex items-baseline gap-3 rounded-md px-2 py-1.5 text-sm hover:bg-muted/40">
              <span className="w-40 shrink-0 text-xs text-muted-foreground">
                {new Date(e.created_at).toLocaleString()}
              </span>
              <Badge variant="outline">{e.kind}</Badge>
              <span className="truncate text-xs text-muted-foreground">
                {summary(e.kind, e.payload)}
              </span>
            </div>
          )
          return e.run_id ? (
            <Link key={e.id} to={`/runs/${e.run_id}`} className="block">
              {row}
            </Link>
          ) : (
            <div key={e.id}>{row}</div>
          )
        })}
      </div>
      {data && data.events.length === 0 && (
        <p className="text-sm text-muted-foreground">Nothing has happened yet.</p>
      )}
    </>
  )
}

function summary(kind: string, payload: Record<string, unknown>): string {
  if (kind === 'run.started') return String(payload.workflow_name ?? '')
  if (kind === 'artifact.emitted') return String(payload.name ?? '')
  if (kind === 'run.failed') return String(payload.error ?? '')
  if (kind === 'run.aborted') return `by ${String(payload.by ?? '?')}`
  return ''
}
