import { Link, useParams } from 'react-router-dom'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { ArrowLeft, Play } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Switch } from '@/components/ui/switch'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'
import { workflowsApi } from '@/lib/api'

const STATUS_VARIANT: Record<string, 'default' | 'secondary' | 'destructive'> = {
  completed: 'default',
  running: 'secondary',
  failed: 'destructive',
}

export function WorkflowDetailPage() {
  const { id } = useParams<{ id: string }>()
  const qc = useQueryClient()
  const { data } = useQuery({
    queryKey: ['workflow', id],
    queryFn: () => workflowsApi.get(id!),
    enabled: !!id,
  })
  const { data: runsData } = useQuery({
    queryKey: ['workflow-runs', id],
    queryFn: () => workflowsApi.runs(id!),
    enabled: !!id,
    refetchInterval: 3000,
  })

  const setActive = useMutation({
    mutationFn: (active: boolean) => workflowsApi.setActive(id!, active),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['workflow', id] }),
  })
  const fire = useMutation({
    mutationFn: () => workflowsApi.fire(id!),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['workflow-runs', id] }),
  })

  const wf = data?.workflow
  if (!wf) return <p className="text-sm text-muted-foreground">Loading...</p>

  return (
    <>
      <Link
        to="/workflows"
        className="inline-flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
      >
        <ArrowLeft className="h-4 w-4" /> Workflows
      </Link>
      <div className="flex items-center justify-between">
        <h1 className="text-xl font-bold tracking-tight">{wf.name}</h1>
        <div className="flex items-center gap-3">
          <label className="flex items-center gap-2 text-sm text-muted-foreground">
            Active
            <Switch
              checked={wf.active}
              onCheckedChange={(v: boolean) => setActive.mutate(v)}
            />
          </label>
          <Button
            disabled={!wf.active || fire.isPending}
            onClick={() => fire.mutate()}
            title={wf.active ? 'Fire the manual trigger' : 'Activate first'}
          >
            <Play className="mr-1 h-4 w-4" /> Run
          </Button>
        </div>
      </div>

      <Card>
        <CardHeader>
          <CardTitle className="text-base">Stages</CardTitle>
        </CardHeader>
        <CardContent className="space-y-3">
          {wf.definition.map((stage, i) => (
            <div key={i} className="rounded-md border p-3 text-sm">
              <div className="mb-1 flex items-center gap-2">
                <Badge variant="outline">{stage.kind}</Badge>
                {stage.model && (
                  <span className="font-mono text-xs text-muted-foreground">
                    {stage.model}
                  </span>
                )}
              </div>
              {stage.system_prompt && (
                <p className="text-muted-foreground">system: {stage.system_prompt}</p>
              )}
              <p>{stage.user_prompt}</p>
            </div>
          ))}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="text-base">Runs</CardTitle>
        </CardHeader>
        <CardContent>
          {runsData && runsData.runs.length === 0 && (
            <p className="text-sm text-muted-foreground">No runs yet.</p>
          )}
          {runsData && runsData.runs.length > 0 && (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Run</TableHead>
                  <TableHead>Status</TableHead>
                  <TableHead>Started</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {runsData.runs.map((run) => (
                  <TableRow key={run.id}>
                    <TableCell>
                      <Link
                        to={`/runs/${run.id}`}
                        className="font-mono text-xs hover:underline"
                      >
                        {run.id.slice(0, 16)}…
                      </Link>
                    </TableCell>
                    <TableCell>
                      <Badge variant={STATUS_VARIANT[run.status] ?? 'secondary'}>
                        {run.status}
                      </Badge>
                    </TableCell>
                    <TableCell className="text-xs text-muted-foreground">
                      {new Date(run.started_at).toLocaleString()}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          )}
        </CardContent>
      </Card>
    </>
  )
}
