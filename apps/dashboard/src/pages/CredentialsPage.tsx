import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Trash2 } from 'lucide-react'
import { Button } from '@/components/ui/button'
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { api } from '@/lib/api'
import { useWorkspace } from '@/lib/workspace'

/**
 * Workspace credentials (encrypted at rest, injected into agent
 * sessions as env vars). Add CLAUDE_CODE_OAUTH_TOKEN here to run
 * agents.
 */
export function CredentialsPage() {
  const ws = useWorkspace()
  const qc = useQueryClient()
  const { data } = useQuery({
    queryKey: ['credentials', ws.id],
    queryFn: () => api.listCredentials(ws.id),
  })
  const [name, setName] = useState('CLAUDE_CODE_OAUTH_TOKEN')
  const [value, setValue] = useState('')

  const invalidate = () => qc.invalidateQueries({ queryKey: ['credentials', ws.id] })
  const save = useMutation({
    mutationFn: () => api.setCredential(ws.id, name.trim(), value),
    onSuccess: () => {
      setValue('')
      invalidate()
    },
  })
  const remove = useMutation({
    mutationFn: (credName: string) => api.deleteCredential(ws.id, credName),
    onSuccess: invalidate,
  })

  return (
    <>
      <h1 className="text-xl font-bold tracking-tight">Credentials</h1>
      <Card>
        <CardHeader>
          <CardTitle className="text-base">Add credential</CardTitle>
          <CardDescription>
            Encrypted at rest; handed to agent sessions as an env var. Values are never
            shown again.
          </CardDescription>
        </CardHeader>
        <CardContent className="flex gap-2">
          <Input
            placeholder="NAME"
            value={name}
            onChange={(e) => setName(e.target.value)}
            className="max-w-xs font-mono"
          />
          <Input
            placeholder="value"
            type="password"
            value={value}
            onChange={(e) => setValue(e.target.value)}
          />
          <Button
            disabled={save.isPending || !name.trim() || !value}
            onClick={() => save.mutate()}
          >
            Save
          </Button>
        </CardContent>
      </Card>
      <div className="space-y-2">
        {data?.credentials.map((c) => (
          <div
            key={c.name}
            className="flex items-center justify-between rounded-md border px-3 py-2 text-sm"
          >
            <span className="font-mono">{c.name}</span>
            <div className="flex items-center gap-3">
              <span className="text-xs text-muted-foreground">
                updated {new Date(c.updated_at).toLocaleString()}
              </span>
              <Button
                variant="ghost"
                size="sm"
                aria-label={`Delete ${c.name}`}
                onClick={() => remove.mutate(c.name)}
              >
                <Trash2 className="h-4 w-4" />
              </Button>
            </div>
          </div>
        ))}
        {data && data.credentials.length === 0 && (
          <p className="text-sm text-muted-foreground">No credentials yet.</p>
        )}
      </div>
    </>
  )
}
