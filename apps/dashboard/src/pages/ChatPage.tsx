import {
  type FormEvent,
  type KeyboardEvent,
  type RefObject,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react"
import { AlertTriangle, MessageSquare, Send, Sparkles } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Textarea } from "@/components/ui/textarea"
import { HitlCard } from "@/components/chat/HitlCard"
import type { HitlCard as HitlCardSpec, HitlCardState } from "@/components/chat/hitl-types"
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
import { LlmConfigError, runChatTurn, type LlmTurnMessage } from "@/lib/chat/llm-client"
import {
  chatStorageKey,
  loadStoredTurns,
  saveStoredTurns,
  type StoredTurn,
} from "@/lib/chat/chat-storage"
import { useActiveWorkspace } from "@/lib/workspace"
import { useAuth } from "@/lib/auth"
import { usePageHeader } from "@/components/layout/PageHeader"

// ─── Runtime types ──────────────────────────────────────────────────────────

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
  resultText?: string
}

interface ChatTurn {
  id: string
  userMessage: ChatMessage
  assistantMessage?: ChatMessage
  toolCalls: ToolCallEntry[]
  error?: string
}

// ─── Serialization ──────────────────────────────────────────────────────────

function hydrateTurns(stored: StoredTurn[]): ChatTurn[] {
  return stored.map((s) => ({
    id: s.id,
    userMessage: { id: `${s.id}-u`, role: "user", content: s.userContent },
    assistantMessage: s.assistantContent
      ? { id: `${s.id}-a`, role: "assistant", content: s.assistantContent }
      : undefined,
    toolCalls: s.toolCalls.map((tc) => ({
      id: tc.id,
      turnId: s.id,
      binding: findMcpTool(tc.toolName),
      toolName: tc.toolName,
      input: tc.input,
      card: buildCardFor(tc.toolName, tc.input),
      state: tc.state,
      errorMessage: tc.errorMessage,
      resultText: tc.resultText,
    })),
    error: s.error,
  }))
}

function dehydrateTurns(turns: ChatTurn[]): StoredTurn[] {
  return turns.map((t) => ({
    id: t.id,
    userContent: t.userMessage.content,
    assistantContent: t.assistantMessage?.content,
    toolCalls: t.toolCalls.map((tc) => ({
      id: tc.id,
      toolName: tc.toolName,
      input: tc.input,
      state: tc.state,
      errorMessage: tc.errorMessage,
      resultText: tc.resultText,
    })),
    error: t.error,
  }))
}

// ─── Helpers ────────────────────────────────────────────────────────────────

function buildCardFor(
  name: string,
  input: Record<string, unknown>,
): HitlCardSpec | undefined {
  return findMcpTool(name)?.buildCard?.(input)
}

function applySupersession(turns: ChatTurn[], newCalls: ToolCallEntry[]): ChatTurn[] {
  const newMutationNames = new Set(
    newCalls.filter((c) => isMutationTool(c.toolName)).map((c) => c.toolName),
  )
  if (newMutationNames.size === 0) return turns
  return turns.map((turn) => ({
    ...turn,
    toolCalls: turn.toolCalls.map((call) => {
      if (!isMutationTool(call.toolName)) return call
      if (!newMutationNames.has(call.toolName)) return call
      if (call.state !== "pending" && call.state !== "failed") return call
      return { ...call, state: "superseded" as HitlCardState }
    }),
  }))
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

function findCall(
  turns: ChatTurn[],
  turnId: string,
  callId: string,
): ToolCallEntry | undefined {
  return turns.find((t) => t.id === turnId)?.toolCalls.find((c) => c.id === callId)
}

function newId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return crypto.randomUUID()
  }
  return `id_${Math.random().toString(36).slice(2)}_${Date.now()}`
}

function errMsg(err: unknown): string {
  if (err instanceof McpClientError) return `MCP error (${err.code}): ${err.message}`
  if (err instanceof Error) return err.message
  return String(err)
}

// ─── ChatPage ───────────────────────────────────────────────────────────────

export function ChatPage() {
  usePageHeader({ title: "Chat" })

  const workspace = useActiveWorkspace()
  const { user } = useAuth()

  const storageKey = useMemo(
    () => (user ? chatStorageKey(user.id, workspace.id) : null),
    [user, workspace.id],
  )

  const [tools, setTools] = useState<McpTool[] | null>(null)
  const [toolsError, setToolsError] = useState<string | null>(null)
  const [submitting, setSubmitting] = useState(false)
  const [prompt, setPrompt] = useState("")
  const feedEndRef = useRef<HTMLDivElement>(null)

  // Load conversation from localStorage on mount. user is guaranteed
  // non-null behind ProtectedRoute; compute key inline to avoid a
  // second render before the initializer fires.
  const [turns, setTurns] = useState<ChatTurn[]>(() => {
    if (!user) return []
    return hydrateTurns(loadStoredTurns(chatStorageKey(user.id, workspace.id)))
  })

  // Persist on every turn change.
  useEffect(() => {
    if (!storageKey) return
    saveStoredTurns(storageKey, dehydrateTurns(turns))
  }, [turns, storageKey])

  // Load MCP tools once.
  useEffect(() => {
    let cancelled = false
    mcpListTools()
      .then((t) => {
        if (!cancelled) setTools(t)
      })
      .catch((e: unknown) => {
        if (!cancelled) setToolsError(errMsg(e))
      })
    return () => {
      cancelled = true
    }
  }, [])

  // Scroll new content into view.
  useEffect(() => {
    feedEndRef.current?.scrollIntoView({ behavior: "smooth" })
  }, [turns])

  const llmMessages = useMemo<LlmTurnMessage[]>(() => {
    const out: LlmTurnMessage[] = [
      { role: "user", content: `(context) workspace_id = ${workspace.id}` },
      { role: "assistant", content: "Acknowledged." },
    ]
    for (const t of turns) {
      out.push({ role: "user", content: t.userMessage.content })
      if (t.assistantMessage) {
        out.push({ role: "assistant", content: t.assistantMessage.content })
      }
    }
    return out
  }, [turns, workspace.id])

  const autoRunReadOnly = useCallback(async (turnId: string, call: ToolCallEntry) => {
    try {
      const result = await mcpCallTool(call.toolName, call.input)
      if (result.isError) {
        const text = result.content[0]?.text ?? "tool returned isError=true"
        setTurns((prev) => updateCall(prev, turnId, call.id, { state: "failed", errorMessage: text }))
        return
      }
      setTurns((prev) =>
        updateCall(prev, turnId, call.id, { state: "committed", resultText: result.content[0]?.text }),
      )
    } catch (err) {
      setTurns((prev) => updateCall(prev, turnId, call.id, { state: "failed", errorMessage: errMsg(err) }))
    }
  }, [])

  const doSubmit = useCallback(async () => {
    const text = prompt.trim()
    if (!text || submitting || !tools) return
    setPrompt("")
    setSubmitting(true)
    const turnId = newId()
    const userMessage: ChatMessage = { id: `${turnId}-u`, role: "user", content: text }
    setTurns((prev) => [...prev, { id: turnId, userMessage, toolCalls: [] }])
    try {
      const result = await runChatTurn({
        messages: [...llmMessages, { role: "user", content: text }],
        tools,
        workspaceId: workspace.id,
      })
      const assistantMessage: ChatMessage | undefined = result.text
        ? { id: `${turnId}-a`, role: "assistant", content: result.text }
        : undefined
      const newToolCalls: ToolCallEntry[] = result.toolCalls.map((tc) => ({
        id: tc.id,
        turnId,
        binding: findMcpTool(tc.name),
        toolName: tc.name,
        input: tc.input,
        card: buildCardFor(tc.name, tc.input),
        state: isMutationTool(tc.name) ? "pending" : ("committing" as HitlCardState),
      }))
      setTurns((prev) => {
        const withSupersession = applySupersession(prev, newToolCalls)
        return withSupersession.map((t) =>
          t.id === turnId ? { ...t, assistantMessage, toolCalls: newToolCalls } : t,
        )
      })
      for (const tc of newToolCalls) {
        if (isMutationTool(tc.toolName)) continue
        autoRunReadOnly(turnId, tc)
      }
    } catch (err) {
      const msg = err instanceof LlmConfigError ? err.message : errMsg(err)
      setTurns((prev) => prev.map((t) => (t.id === turnId ? { ...t, error: msg } : t)))
    } finally {
      setSubmitting(false)
    }
  }, [prompt, submitting, tools, llmMessages, workspace.id, autoRunReadOnly])

  const onFormSubmit = useCallback(
    (e: FormEvent) => {
      e.preventDefault()
      doSubmit()
    },
    [doSubmit],
  )

  const onKeyDown = useCallback(
    (e: KeyboardEvent<HTMLTextAreaElement>) => {
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault()
        doSubmit()
      }
    },
    [doSubmit],
  )

  const handleCommit = useCallback(
    async (turnId: string, callId: string, editedValues: Record<string, string>) => {
      setTurns((prev) =>
        updateCall(prev, turnId, callId, { state: "committing", errorMessage: undefined }),
      )
      const target = findCall(turns, turnId, callId)
      if (!target) return
      const mergedArgs = { ...target.input, ...editedValues }
      try {
        const result = await mcpCallTool(target.toolName, mergedArgs)
        if (result.isError) {
          const text = result.content[0]?.text ?? "tool returned isError=true"
          setTurns((prev) => updateCall(prev, turnId, callId, { state: "failed", errorMessage: text }))
          return
        }
        setTurns((prev) =>
          updateCall(prev, turnId, callId, { state: "committed", resultText: result.content[0]?.text }),
        )
      } catch (err) {
        setTurns((prev) =>
          updateCall(prev, turnId, callId, { state: "failed", errorMessage: errMsg(err) }),
        )
      }
    },
    [turns],
  )

  const handleReject = useCallback((turnId: string, callId: string) => {
    setTurns((prev) => updateCall(prev, turnId, callId, { state: "rejected" }))
  }, [])

  const isEmpty = turns.length === 0

  return (
    <div className="flex min-h-full flex-col">
      <div className="mb-6 hidden md:block">
        <h1 className="text-2xl font-bold tracking-tight">Chat</h1>
        <p className="text-sm text-muted-foreground">
          R&amp;D mode — design workflows by talking to the agent.
        </p>
      </div>

      <div className="flex-1">
        {isEmpty ? (
          <EmptyState onChip={setPrompt} />
        ) : (
          <ConversationFeed
            turns={turns}
            handleCommit={handleCommit}
            handleReject={handleReject}
            feedEndRef={feedEndRef}
          />
        )}
      </div>

      {/* Sticky composer — breaks out of main's horizontal padding to span full
          width; background covers content below when pinned at viewport bottom. */}
      <div className="sticky bottom-0 -mx-4 border-t bg-background/95 px-4 py-3 backdrop-blur supports-[backdrop-filter]:bg-background/80 md:-mx-6 md:px-6 md:py-4">
        {toolsError ? (
          <div
            role="alert"
            className="mb-2 flex items-start gap-2 rounded-md border border-destructive/40 bg-destructive/5 p-2 text-xs text-destructive"
          >
            <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
            <span>MCP server unavailable — {toolsError}</span>
          </div>
        ) : null}
        <form onSubmit={onFormSubmit} className="flex items-end gap-2">
          <Textarea
            value={prompt}
            onChange={(e) => setPrompt(e.target.value)}
            onKeyDown={onKeyDown}
            placeholder={
              isEmpty
                ? "e.g. Design a workflow that runs on every issue comment."
                : "Refine or ask a follow-up…"
            }
            rows={2}
            disabled={!tools || submitting}
            className="flex-1 resize-none"
            aria-label="Describe what you want the agent to do"
          />
          <Button
            type="submit"
            size="sm"
            disabled={!prompt.trim() || submitting || !tools}
            aria-label="Send message"
          >
            <Send className="h-4 w-4" />
            {submitting ? "Thinking…" : "Send"}
          </Button>
        </form>
        <div className="mt-1.5 text-xs text-muted-foreground">
          {tools
            ? `${tools.length} tools available`
            : toolsError
              ? "MCP disconnected"
              : "Connecting to MCP server…"}
          {" · "}
          ⏎ to send · Shift+⏎ for new line
        </div>
      </div>
    </div>
  )
}

// ─── Empty state ─────────────────────────────────────────────────────────────

const EXAMPLE_CHIPS = [
  "Create a workflow triggered by GitHub issue comments",
  "List all my workflows",
  "Show me recent runs and their status",
]

function EmptyState({ onChip }: { onChip: (text: string) => void }) {
  return (
    <div className="flex flex-col items-center justify-center gap-6 py-16 text-center">
      <div className="flex h-14 w-14 items-center justify-center rounded-full bg-primary/10 text-primary">
        <MessageSquare className="h-7 w-7" />
      </div>
      <div className="flex flex-col gap-2">
        <h2 className="text-2xl font-bold tracking-tight">Design something.</h2>
        <p className="max-w-sm text-sm text-muted-foreground">
          Describe what you want to build. The agent will propose workflows,
          runs, and changes — you review every step before anything is applied.
        </p>
      </div>
      <div className="flex flex-wrap justify-center gap-2">
        {EXAMPLE_CHIPS.map((chip) => (
          <Button
            key={chip}
            type="button"
            variant="outline"
            size="sm"
            className="rounded-full"
            onClick={() => onChip(chip)}
          >
            {chip}
          </Button>
        ))}
      </div>
      <p className="max-w-xs text-xs text-muted-foreground">
        <Sparkles className="mr-1 inline h-3 w-3" />
        Every change requires your approval before it&apos;s applied.
      </p>
    </div>
  )
}

// ─── Conversation feed ────────────────────────────────────────────────────────

interface ConversationFeedProps {
  turns: ChatTurn[]
  handleCommit: (turnId: string, callId: string, edits: Record<string, string>) => void
  handleReject: (turnId: string, callId: string) => void
  feedEndRef: RefObject<HTMLDivElement | null>
}

function ConversationFeed({
  turns,
  handleCommit,
  handleReject,
  feedEndRef,
}: ConversationFeedProps) {
  return (
    <ol className="flex flex-col gap-4 pb-4">
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
                  body={`The agent invoked \`${call.toolName}\`, which is not in the dashboard registry.`}
                />
              )
            }
            if (call.card && isMutationTool(call.toolName)) {
              return (
                <div key={call.id} className="flex flex-col gap-1.5">
                  <HitlCard
                    card={call.card}
                    state={call.state}
                    errorMessage={call.errorMessage}
                    onCommit={(edits) => handleCommit(turn.id, call.id, edits)}
                    onReject={() => handleReject(turn.id, call.id)}
                  />
                  {call.state === "committed" && call.resultText ? (
                    <ResultBlock text={call.resultText} />
                  ) : null}
                </div>
              )
            }
            return (
              <InfoBlock
                key={call.id}
                title={call.binding.title(call.input)}
                body={call.binding.renderInfo?.(call.input) ?? `Calling ${call.toolName}.`}
                state={call.state}
                resultText={call.resultText}
                errorMessage={call.errorMessage}
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
      <div ref={feedEndRef} />
    </ol>
  )
}

// ─── Bubbles and blocks ───────────────────────────────────────────────────────

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

function InfoBlock({
  title,
  body,
  state,
  resultText,
  errorMessage,
}: {
  title: string
  body: string
  state?: HitlCardState
  resultText?: string
  errorMessage?: string
}) {
  const statusLabel =
    state === "committing" ? "Running…" : state === "failed" ? "Failed" : undefined
  return (
    <div
      data-slot="mcp-info-block"
      data-state={state}
      className="flex flex-col gap-1 rounded-md border bg-muted/20 px-2.5 py-1.5"
    >
      <div className="flex items-center gap-2">
        <div className="text-xs font-medium">{title}</div>
        {statusLabel ? (
          <span className="text-xs italic text-muted-foreground">{statusLabel}</span>
        ) : null}
      </div>
      <div className="text-xs text-muted-foreground">{body}</div>
      {state === "failed" && errorMessage ? (
        <div className="text-xs text-destructive">{errorMessage}</div>
      ) : null}
      {state === "committed" && resultText ? <ResultBlock text={resultText} /> : null}
    </div>
  )
}

function ResultBlock({ text }: { text: string }) {
  return (
    <pre
      data-slot="mcp-tool-result"
      className="max-h-48 overflow-auto whitespace-pre-wrap break-all rounded-md border bg-background/60 p-2 font-mono text-[11px]"
    >
      {text}
    </pre>
  )
}
