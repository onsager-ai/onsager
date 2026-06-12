import { useQuery } from '@tanstack/react-query'
import Markdown from 'react-markdown'
import { AlertTriangle, ChevronRight, Wrench } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { sessionsApi, type SessionEvent } from '@/lib/api'

/**
 * The agent session feed (#632) — what the agent said and did, as one
 * chronological feed. Design ported from arbor's ActivityFeed: prose
 * renders as markdown, tool_use/tool_result pairs fold into collapsible
 * cards with running/done/error state, warnings become callouts,
 * lifecycle events are quiet rows. Polls while the session runs.
 */
export function SessionFeed({ sessionId, status }: { sessionId: string; status: string }) {
  const { data } = useQuery({
    queryKey: ['session-events', sessionId],
    queryFn: () => sessionsApi.events(sessionId),
    refetchInterval: (q) => (q.state.data?.status === 'running' ? 2000 : false),
  })
  const events = data?.events ?? []
  const live = (data?.status ?? status) === 'running'
  const items = foldToolPairs(events)

  return (
    <Card>
      <CardHeader>
        <div className="flex items-center gap-2">
          <CardTitle className="text-base">Agent session</CardTitle>
          <span className="font-mono text-xs text-muted-foreground">
            {sessionId.slice(0, 18)}…
          </span>
          <Badge variant={live ? 'secondary' : 'outline'}>{data?.status ?? status}</Badge>
        </div>
      </CardHeader>
      <CardContent className="space-y-2">
        {items.length === 0 && (
          <p className="text-sm italic text-muted-foreground">
            {live ? 'waiting for the agent…' : 'no session events recorded'}
          </p>
        )}
        {items.map((item) => renderItem(item))}
        {live && (
          <span aria-hidden className="ml-1 inline-block animate-pulse text-primary">
            ▍
          </span>
        )}
      </CardContent>
    </Card>
  )
}

type ToolItem = {
  kind: 'tool'
  seq: number
  tool: string
  inputExcerpt: string
  result?: { excerpt: string; isError: boolean }
}
type FeedItem = { kind: 'event'; event: SessionEvent } | ToolItem

/** Pair each tool_use with its tool_result (by tool_use_id) into one card. */
function foldToolPairs(events: SessionEvent[]): FeedItem[] {
  const items: FeedItem[] = []
  const open = new Map<string, ToolItem>()
  for (const e of events) {
    if (e.kind === 'agent.tool_use') {
      const item: ToolItem = {
        kind: 'tool',
        seq: e.seq,
        tool: String(e.payload.tool ?? 'unknown'),
        inputExcerpt: String(e.payload.input_excerpt ?? ''),
      }
      items.push(item)
      const id = e.payload.tool_use_id
      if (typeof id === 'string') open.set(id, item)
    } else if (e.kind === 'agent.tool_result') {
      const id = e.payload.tool_use_id
      const target = typeof id === 'string' ? open.get(id) : undefined
      const result = {
        excerpt: String(e.payload.result_excerpt ?? ''),
        isError: e.payload.is_error === true,
      }
      if (target && !target.result) {
        target.result = result
        if (typeof id === 'string') open.delete(id)
      } else {
        // Orphan result — surfaced, never dropped (arbor convention).
        items.push({
          kind: 'tool',
          seq: e.seq,
          tool: String(e.payload.tool ?? 'unknown'),
          inputExcerpt: '',
          result,
        })
      }
    } else {
      items.push({ kind: 'event', event: e })
    }
  }
  return items
}

function renderItem(item: FeedItem) {
  if (item.kind === 'tool') return <ToolCard key={`t${item.seq}`} item={item} />
  const e = item.event
  switch (e.kind) {
    case 'agent.text':
      return (
        <div key={e.seq} className="prose prose-sm max-w-none text-sm leading-relaxed dark:prose-invert">
          <Markdown>{String(e.payload.text ?? '')}</Markdown>
        </div>
      )
    case 'agent.warning':
      return (
        <div
          key={e.seq}
          className="flex items-start gap-2 rounded-md border border-amber-300 bg-amber-50 px-3 py-2 text-sm text-amber-900 dark:border-amber-900 dark:bg-amber-950/40 dark:text-amber-200"
        >
          <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
          <span>{String(e.payload.message ?? '')}</span>
        </div>
      )
    case 'agent.started':
      return (
        <p key={e.seq} className="text-xs italic text-muted-foreground">
          session started{e.payload.model ? ` · ${String(e.payload.model)}` : ''}
        </p>
      )
    case 'agent.completed': {
      const bits = [
        e.payload.turns != null ? `${String(e.payload.turns)} turns` : null,
        e.payload.duration_ms != null
          ? `${(Number(e.payload.duration_ms) / 1000).toFixed(1)}s`
          : null,
        e.payload.cost_usd != null ? `$${Number(e.payload.cost_usd).toFixed(4)}` : null,
      ].filter(Boolean)
      return (
        <p key={e.seq} className="text-xs italic text-muted-foreground">
          session completed{bits.length > 0 ? ` · ${bits.join(' · ')}` : ''}
        </p>
      )
    }
    default:
      return null
  }
}

function ToolCard({ item }: { item: ToolItem }) {
  const state = item.result === undefined ? 'running' : item.result.isError ? 'error' : 'done'
  return (
    <details
      className={`group overflow-hidden rounded-lg border ${
        state === 'error' ? 'border-red-300 dark:border-red-900' : ''
      } bg-muted/30`}
    >
      <summary className="flex cursor-pointer list-none items-center gap-2 px-2.5 py-1.5 text-sm">
        <ChevronRight className="h-3.5 w-3.5 shrink-0 text-muted-foreground transition-transform group-open:rotate-90" />
        <Wrench className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        <span className="font-mono text-xs font-semibold">{item.tool}</span>
        <span className="min-w-0 flex-1 truncate font-mono text-[0.7rem] text-muted-foreground">
          {item.inputExcerpt}
        </span>
        <span
          className={`shrink-0 text-[0.7rem] ${
            state === 'running'
              ? 'text-blue-600 dark:text-blue-400'
              : state === 'error'
                ? 'text-red-600 dark:text-red-400'
                : 'text-emerald-600 dark:text-emerald-400'
          }`}
        >
          {state === 'running' ? 'running…' : state}
        </span>
      </summary>
      <div className="space-y-1.5 px-2.5 pb-2">
        {item.inputExcerpt && (
          <pre className="overflow-x-auto whitespace-pre-wrap break-words rounded-md bg-background px-2 py-1.5 font-mono text-[0.72rem]">
            {item.inputExcerpt}
          </pre>
        )}
        {item.result?.excerpt && (
          <pre
            className={`overflow-x-auto whitespace-pre-wrap break-words rounded-md px-2 py-1.5 font-mono text-[0.72rem] ${
              item.result.isError
                ? 'bg-red-50 text-red-900 dark:bg-red-950/40 dark:text-red-200'
                : 'bg-background'
            }`}
          >
            {item.result.excerpt}
          </pre>
        )}
      </div>
    </details>
  )
}
