import { type FormEvent, useCallback, useEffect, useMemo, useState } from "react"
import { AlertTriangle, Send, Sparkles } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Card, CardContent } from "@/components/ui/card"
import { Textarea } from "@/components/ui/textarea"
import { HitlCard } from "@/components/chat/HitlCard"
import type {
  HitlCard as HitlCardSpec,
  HitlCardState,
} from "@/components/chat/hitl-types"
import {
  McpClientError,
  mcpCallTool,
  mcpListTools,
  type McpTool,
} from "@/lib/mcp-client"
import {
  findMcpTool,
  isMutationTool,
  type McpToolBinding,
} from "@/lib/mcp-tools"
import {
  LlmConfigError,
  runChatTurn,
  type LlmTurnMessage,
} from "@/lib/chat/llm-client"

export interface ChatBuilderProps {
  /**
   * Workspace the chat is scoped to. Passed to the LLM as context so it
   * can fill `workspace_id` arguments without asking. Optional —
   * #289 PR 4 promotes ChatBuilder to a top-level surface and threads
   * workspace context in from the route.
   */
  workspaceId?: string
}

interface ChatMessage {
  id: string
  role: "user" | "assistant"
  content: string
}

interface ToolCallEntry {
  id: string
  turnId: string
  binding: McpToolBinding | undefined
  toolName: string
  input: Record<string, unknown>
  card?: HitlCardSpec
  state: HitlCardState
  errorMessage?: string
  /** Tool result text (for committed mutations or auto-run read-only calls). */
  resultText?: string
}

interface ChatTurn {
  id: string
  userMessage: ChatMessage
  assistantMessage?: ChatMessage
  toolCalls: ToolCallEntry[]
  error?: string
}

/**
 * Same-origin MCP client embedded as a workflow-builder chat. Replaces
 * the stub LLM with an Anthropic SDK call (latest Claude model + prompt
 * caching). Every mutation tool call surfaces as a HitlCard the user
 * commits or rejects; read-only tools render as plain info blocks.
 *
 * Per spec #311, ChatBuilder stays a self-contained component for this
 * spec — #289 PR 4 promotes it to a top-level surface and adds the
 * first-run / refinement chat-orchestration UX on top.
 */
export function ChatBuilder({ workspaceId }: ChatBuilderProps) {
  const [prompt, setPrompt] = useState("")
  const [turns, setTurns] = useState<ChatTurn[]>([])
  const [tools, setTools] = useState<McpTool[] | null>(null)
  const [toolsError, setToolsError] = useState<string | null>(null)
  const [submitting, setSubmitting] = useState(false)

  // Load the tool registry on mount. The dashboard talks same-origin
  // to `/mcp/messages`; cookies ride along.
  useEffect(() => {
    let cancelled = false
    mcpListTools()
      .then((t) => {
        if (!cancelled) setTools(t)
      })
      .catch((e: unknown) => {
        if (!cancelled) setToolsError(errorMessage(e))
      })
    return () => {
      cancelled = true
    }
  }, [])

  const llmMessages = useMemo<LlmTurnMessage[]>(() => {
    const out: LlmTurnMessage[] = []
    if (workspaceId) {
      out.push({
        role: "user",
        content: `(context) workspace_id = ${workspaceId}`,
      })
      out.push({ role: "assistant", content: "Acknowledged." })
    }
    for (const t of turns) {
      out.push({ role: "user", content: t.userMessage.content })
      if (t.assistantMessage) {
        out.push({ role: "assistant", content: t.assistantMessage.content })
      }
    }
    return out
  }, [turns, workspaceId])

  const onSubmit = useCallback(
    async (e: FormEvent) => {
      e.preventDefault()
      const text = prompt.trim()
      if (!text || submitting || !tools) return
      setPrompt("")
      setSubmitting(true)
      const turnId = newId()
      const userMessage: ChatMessage = {
        id: `${turnId}-u`,
        role: "user",
        content: text,
      }
      setTurns((prev) => [
        ...prev,
        { id: turnId, userMessage, toolCalls: [] },
      ])
      try {
        const result = await runChatTurn({
          messages: [...llmMessages, { role: "user", content: text }],
          tools,
          workspaceId,
        })
        const assistantMessage: ChatMessage | undefined = result.text
          ? { id: `${turnId}-a`, role: "assistant", content: result.text }
          : undefined
        const toolCalls: ToolCallEntry[] = result.toolCalls.map((tc) => ({
          id: tc.id,
          turnId,
          binding: findMcpTool(tc.name),
          toolName: tc.name,
          input: tc.input,
          card: buildCardFor(tc.name, tc.input),
          state: isMutationTool(tc.name) ? "pending" : "committed",
        }))
        setTurns((prev) =>
          prev.map((t) =>
            t.id === turnId
              ? { ...t, assistantMessage, toolCalls }
              : t,
          ),
        )
      } catch (err) {
        const msg =
          err instanceof LlmConfigError
            ? `${err.message} Set VITE_ANTHROPIC_API_KEY and reload the dashboard.`
            : errorMessage(err)
        setTurns((prev) =>
          prev.map((t) => (t.id === turnId ? { ...t, error: msg } : t)),
        )
      } finally {
        setSubmitting(false)
      }
    },
    [prompt, submitting, tools, llmMessages, workspaceId],
  )

  const handleCommit = useCallback(
    async (
      turnId: string,
      callId: string,
      editedValues: Record<string, string>,
    ) => {
      setTurns((prev) =>
        prev.map((t) =>
          t.id === turnId
            ? {
                ...t,
                toolCalls: t.toolCalls.map((c) =>
                  c.id === callId
                    ? { ...c, state: "committing", errorMessage: undefined }
                    : c,
                ),
              }
            : t,
        ),
      )
      const target = findCall(turns, turnId, callId)
      if (!target) return
      const mergedArgs = { ...target.input, ...editedValues }
      try {
        const result = await mcpCallTool(target.toolName, mergedArgs)
        if (result.isError) {
          const text = result.content[0]?.text ?? "tool returned isError=true"
          setTurns((prev) => updateCall(prev, turnId, callId, {
            state: "failed",
            errorMessage: text,
          }))
          return
        }
        const resultText = result.content[0]?.text
        setTurns((prev) => updateCall(prev, turnId, callId, {
          state: "committed",
          resultText,
        }))
      } catch (err) {
        setTurns((prev) => updateCall(prev, turnId, callId, {
          state: "failed",
          errorMessage: errorMessage(err),
        }))
      }
    },
    [turns],
  )

  const handleReject = useCallback((turnId: string, callId: string) => {
    setTurns((prev) =>
      updateCall(prev, turnId, callId, { state: "rejected" }),
    )
  }, [])

  return (
    <Card>
      <CardContent className="flex flex-col gap-3 p-4">
        <header className="flex items-center gap-2 text-xs text-muted-foreground">
          <Sparkles className="h-3.5 w-3.5" />
          {tools
            ? `Connected to the Onsager MCP server — ${tools.length} tools available.`
            : toolsError
              ? "Could not connect to the MCP server."
              : "Connecting to the MCP server…"}
        </header>
        {toolsError ? (
          <div
            role="alert"
            className="flex items-start gap-2 rounded-md border border-destructive/40 bg-destructive/5 p-2 text-xs text-destructive"
          >
            <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
            <span>{toolsError}</span>
          </div>
        ) : null}

        {turns.length > 0 ? (
          <ol className="flex flex-col gap-3">
            {turns.map((turn) => (
              <li key={turn.id} className="flex flex-col gap-2">
                <UserBubble content={turn.userMessage.content} />
                {turn.assistantMessage ? (
                  <AssistantBubble content={turn.assistantMessage.content} />
                ) : null}
                {turn.toolCalls.map((call) => {
                  if (!call.binding) {
                    return (
                      <InfoBlock
                        key={call.id}
                        title="Unknown tool"
                        body={`The agent invoked \`${call.toolName}\`, which is not in the dashboard's tool registry.`}
                      />
                    )
                  }
                  if (call.card && isMutationTool(call.toolName)) {
                    return (
                      <HitlCard
                        key={call.id}
                        card={call.card}
                        state={call.state}
                        errorMessage={call.errorMessage}
                        onCommit={(edits) =>
                          handleCommit(turn.id, call.id, edits)
                        }
                        onReject={() => handleReject(turn.id, call.id)}
                      />
                    )
                  }
                  return (
                    <InfoBlock
                      key={call.id}
                      title={call.binding.title(call.input)}
                      body={
                        call.binding.renderInfo?.(call.input) ??
                        `Calling ${call.toolName}.`
                      }
                    />
                  )
                })}
                {turn.error ? (
                  <div
                    role="alert"
                    className="rounded-md border border-destructive/40 bg-destructive/5 px-2 py-1.5 text-xs text-destructive"
                  >
                    {turn.error}
                  </div>
                ) : null}
              </li>
            ))}
          </ol>
        ) : null}

        <form onSubmit={onSubmit} className="flex flex-col gap-2">
          <Textarea
            value={prompt}
            onChange={(e) => setPrompt(e.target.value)}
            placeholder="e.g. Design a workflow that runs on every issue comment."
            rows={2}
            disabled={!tools || submitting}
            aria-label="Describe what you want the agent to do"
          />
          <Button
            type="submit"
            disabled={!prompt.trim() || submitting || !tools}
            className="self-end"
          >
            <Send className="h-4 w-4" />
            {submitting ? "Thinking…" : "Send"}
          </Button>
        </form>
      </CardContent>
    </Card>
  )
}

function UserBubble({ content }: { content: string }) {
  return (
    <div className="self-end max-w-[85%] rounded-lg bg-primary/10 px-3 py-1.5 text-sm">
      {content}
    </div>
  )
}

function AssistantBubble({ content }: { content: string }) {
  return (
    <div className="max-w-[85%] rounded-lg bg-muted/40 px-3 py-1.5 text-sm">
      {content}
    </div>
  )
}

function InfoBlock({ title, body }: { title: string; body: string }) {
  return (
    <div
      data-slot="mcp-info-block"
      className="rounded-md border bg-muted/20 px-2.5 py-1.5"
    >
      <div className="text-xs font-medium">{title}</div>
      <div className="text-xs text-muted-foreground">{body}</div>
    </div>
  )
}

function buildCardFor(
  name: string,
  input: Record<string, unknown>,
): HitlCardSpec | undefined {
  const b = findMcpTool(name)
  return b?.buildCard?.(input)
}

function findCall(
  turns: ChatTurn[],
  turnId: string,
  callId: string,
): ToolCallEntry | undefined {
  for (const t of turns) {
    if (t.id !== turnId) continue
    return t.toolCalls.find((c) => c.id === callId)
  }
  return undefined
}

function updateCall(
  turns: ChatTurn[],
  turnId: string,
  callId: string,
  patch: Partial<ToolCallEntry>,
): ChatTurn[] {
  return turns.map((t) =>
    t.id === turnId
      ? {
          ...t,
          toolCalls: t.toolCalls.map((c) =>
            c.id === callId ? { ...c, ...patch } : c,
          ),
        }
      : t,
  )
}

function newId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return crypto.randomUUID()
  }
  return `id_${Math.random().toString(36).slice(2)}_${Date.now()}`
}

function errorMessage(err: unknown): string {
  if (err instanceof McpClientError) return `MCP error (${err.code}): ${err.message}`
  if (err instanceof Error) return err.message
  return String(err)
}
