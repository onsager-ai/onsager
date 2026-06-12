import { useState } from 'react'
import { Link, useNavigate } from 'react-router-dom'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { Textarea } from '@/components/ui/textarea'
import { ApiError, workflowsApi } from '@/lib/api'
import { useWorkspace } from '@/lib/workspace'

/** Workflows list + the builder (manual trigger + one agent stage). */
export function WorkflowsPage() {
  const ws = useWorkspace()
  const qc = useQueryClient()
  const navigate = useNavigate()
  const { data, isLoading } = useQuery({
    queryKey: ['workflows', ws.id],
    queryFn: () => workflowsApi.list(ws.id),
  })

  const [showBuilder, setShowBuilder] = useState(false)
  const [name, setName] = useState('')
  const [triggerKind, setTriggerKind] = useState<'manual' | 'github_issue_webhook'>('manual')
  const [repo, setRepo] = useState('')
  const [label, setLabel] = useState('')
  const [systemPrompt, setSystemPrompt] = useState('')
  const [userPrompt, setUserPrompt] = useState('')
  const [error, setError] = useState<string | null>(null)

  const trigger =
    triggerKind === 'manual'
      ? ({ kind: 'manual', name: 'run' } as const)
      : ({ kind: 'github_issue_webhook', repo, label } as const)
  const triggerValid =
    triggerKind === 'manual' || (repo.includes('/') && label.trim().length > 0)

  const create = useMutation({
    mutationFn: () =>
      workflowsApi.create({
        workspace_id: ws.id,
        name,
        trigger,
        definition: [
          {
            kind: 'agent',
            system_prompt: systemPrompt || null,
            user_prompt: userPrompt,
          },
        ],
      }),
    onSuccess: (res) => {
      qc.invalidateQueries({ queryKey: ['workflows', ws.id] })
      navigate(`/workflows/${res.workflow.id}`)
    },
    onError: (e) =>
      setError(e instanceof ApiError ? e.message : 'failed to create workflow'),
  })

  return (
    <>
      <div className="flex items-center justify-between">
        <h1 className="text-xl font-bold tracking-tight">Workflows</h1>
        <Button onClick={() => setShowBuilder((v) => !v)}>
          {showBuilder ? 'Cancel' : 'New workflow'}
        </Button>
      </div>

      {showBuilder && (
        <Card>
          <CardHeader>
            <CardTitle>New workflow</CardTitle>
            <CardDescription>
              Manual trigger + one agent stage. Fire it from the detail page.
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-3">
            <Input
              placeholder="Workflow name"
              value={name}
              onChange={(e) => setName(e.target.value)}
            />
            <div className="flex items-center gap-2 text-sm">
              <span className="text-muted-foreground">Trigger</span>
              <Button
                type="button"
                variant={triggerKind === 'manual' ? 'default' : 'outline'}
                size="sm"
                onClick={() => setTriggerKind('manual')}
              >
                Manual
              </Button>
              <Button
                type="button"
                variant={triggerKind === 'github_issue_webhook' ? 'default' : 'outline'}
                size="sm"
                onClick={() => setTriggerKind('github_issue_webhook')}
              >
                GitHub issue label
              </Button>
            </div>
            {triggerKind === 'github_issue_webhook' && (
              <div className="flex gap-2">
                <Input
                  placeholder="owner/repo"
                  value={repo}
                  onChange={(e) => setRepo(e.target.value)}
                />
                <Input
                  placeholder="label"
                  value={label}
                  onChange={(e) => setLabel(e.target.value)}
                />
              </div>
            )}
            <Textarea
              placeholder="System prompt (optional)"
              value={systemPrompt}
              onChange={(e) => setSystemPrompt(e.target.value)}
            />
            <Textarea
              placeholder="Task prompt — what should the agent do?"
              value={userPrompt}
              onChange={(e) => setUserPrompt(e.target.value)}
            />
            {error && <p className="text-sm text-destructive">{error}</p>}
            <Button
              disabled={
                create.isPending || !name.trim() || !userPrompt.trim() || !triggerValid
              }
              onClick={() => create.mutate()}
            >
              Create
            </Button>
          </CardContent>
        </Card>
      )}

      {isLoading && <p className="text-sm text-muted-foreground">Loading...</p>}
      {data?.workflows.map((wf) => (
        <Link key={wf.id} to={`/workflows/${wf.id}`} className="block">
          <Card className="transition-colors hover:bg-muted/40">
            <CardHeader>
              <div className="flex items-center justify-between">
                <CardTitle>{wf.name}</CardTitle>
                <Badge variant={wf.active ? 'default' : 'secondary'}>
                  {wf.active ? 'active' : 'inactive'}
                </Badge>
              </div>
              <CardDescription>
                {wf.definition.length} stage{wf.definition.length === 1 ? '' : 's'} ·
                trigger: {wf.trigger.kind}
              </CardDescription>
            </CardHeader>
          </Card>
        </Link>
      ))}
      {data && data.workflows.length === 0 && !showBuilder && (
        <p className="text-sm text-muted-foreground">
          No workflows yet — create one to run an agent.
        </p>
      )}
    </>
  )
}
