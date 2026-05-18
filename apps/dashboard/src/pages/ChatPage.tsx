// budget-allow: ChatPage is the top-level chat surface — it owns the full
// turn lifecycle (persistence, MCP wiring, HITL routing, DAG preview). Its
// breadth cannot be split without a second spec.
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
import Markdown from "react-markdown"
import rehypeHighlight from "rehype-highlight"
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
import {
  readLastUsedWorkspace,
  useMembershipWorkspaces,
  useOptionalActiveWorkspace,
} from "@/lib/workspace"
import { useAuth } from "@/lib/auth"
import { usePageHeader } from "@/components/layout/PageHeader"
import { WorkflowDAGPreview } from "@/components/chat/WorkflowDAGPreview"
import { TemplateGallery } from "@/components/chat/TemplateGallery"
import { DraftStrip } from "@/components/chat/DraftStrip"
import { useWorkflowDraft } from "@/lib/drafts"
import { useBuildInfo } from "@/lib/build-info"
import type {
  WorkflowDocument,
  WorkflowDraft,
} from "@/components/factory/workflows/workflow-draft"
import { type FtueTemplate, templateToDocument } from "@/lib/templates"

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
  return stored.flatMap((s) => {
    if (!s || typeof s !== "object" || !s.id || !s.userContent) return []
    const toolCalls = Array.isArray(s.toolCalls) ? s.toolCalls : []
    return [
      {
        id: s.id,
        userMessage: { id: `${s.id}-u`, role: "user" as const, content: s.userContent },
        assistantMessage: s.assistantContent
          ? { id: `${s.id}-a`, role: "assistant" as const, content: s.assistantContent }
          : undefined,
        toolCalls: toolCalls.flatMap((tc) => {
          if (!tc || !tc.id || !tc.toolName) return []
          const input: Record<string, unknown> =
            tc.input && typeof tc.input === "object" ? (tc.input as Record<string, unknown>) : {}
          return [
            {
              id: tc.id,
              turnId: s.id,
              binding: findMcpTool(tc.toolName),
              toolName: tc.toolName,
              input,
              card: buildCardFor(tc.toolName, input),
              state: (tc.state as HitlCardState) ?? "committed",
              errorMessage: tc.errorMessage as string | undefined,
              resultText: tc.resultText as string | undefined,
            },
          ]
        }),
        error: s.error,
      },
    ]
  })
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

// Extract a WorkflowDocument from a `propose_workflow` /
// `propose_workflow_draft` tool-call's args for the DAG preview. Both
// tools accept the same canonical {name, trigger, stages} shape; the
// draft variant just elides workspace context.
function extractWorkflowDocument(
  args: Record<string, unknown>,
): WorkflowDocument | null {
  try {
    const name = typeof args.name === "string" ? args.name : ""
    const trigger = (args.trigger ?? {}) as Record<string, unknown>
    const stages = Array.isArray(args.stages)
      ? (args.stages as Record<string, unknown>[]).map((s, i) => ({
          id: typeof s.id === "string" ? s.id : `stage-${i}`,
          name: typeof s.name === "string" ? s.name : "",
          gate_kind: (typeof s.gate_kind === "string" ? s.gate_kind : "agent-session") as import("@/lib/api").WorkflowGateKind,
          artifact_kind: (typeof s.artifact_kind === "string" ? s.artifact_kind : "Issue") as import("@/lib/api").WorkflowArtifactKind,
          config: (typeof s.config === "object" && s.config ? s.config : {}) as Record<string, unknown>,
        }))
      : []
    return {
      name,
      trigger: {
        install_id: String(trigger.install_id ?? ""),
        repo_owner: typeof trigger.repo_owner === "string" ? trigger.repo_owner : "",
        repo_name: typeof trigger.repo_name === "string" ? trigger.repo_name : "",
        label: typeof trigger.label === "string" ? trigger.label : "",
      },
      stages,
    }
  } catch {
    return null
  }
}

// ─── ChatPage ───────────────────────────────────────────────────────────────

export function ChatPage() {
  // fullBleed: the chat surface owns the full viewport area below the header;
  // AppLayout switches <main> to overflow-hidden + no padding for split-panel.
  usePageHeader({ title: "Chat", fullBleed: true })

  // The /chat entry is mounted twice — workspace-scoped (under
  // /workspaces/:slug/chat) and unscoped (top-level /chat per spec #398).
  // Tolerate both: prefer the explicit scope, fall back to last-used or
  // memberships[0] so the user's previous workspace is still in scope
  // when they land on /chat without re-typing the URL.
  const scopedWorkspace = useOptionalActiveWorkspace()
  const memberships = useMembershipWorkspaces()
  const workspace = useMemo(() => {
    if (scopedWorkspace) return scopedWorkspace
    const lastUsed = readLastUsedWorkspace()
    if (lastUsed) {
      const match = memberships.find((w) => w.slug === lastUsed)
      if (match) return match
    }
    return memberships[0] ?? null
  }, [scopedWorkspace, memberships])
  // FTUE = truly zero workspace context (not just "visited /chat without
  // a slug"). A returning user on /chat with a resolvable last-used
  // workspace gets the regular surface, not the workspace-less FTUE
  // chrome.
  const isFtue = workspace == null
  const { user } = useAuth()
  const buildInfo = useBuildInfo()
  const isOss = buildInfo?.is_oss ?? false

  // Per spec #401: persist drafts client-side under the user namespace.
  // The active draft drives the right-panel DAG/YAML preview.
  const {
    draft: activeDraft,
    drafts,
    setWorkflow,
    switchDraft,
    newDraft,
    deleteById,
  } = useWorkflowDraft(user?.id ?? null)
  const workflowDoc: WorkflowDocument | null = activeDraft?.workflow ?? null

  // Chat-history scope. Workspace-bound conversations key off the
  // workspace; FTUE conversations bind to the active draft so each draft
  // owns its own back-and-forth, per spec #400.
  const storageKey = useMemo(() => {
    if (!user) return null
    if (workspace) return chatStorageKey(user.id, workspace.id)
    if (activeDraft) return chatStorageKey(user.id, `draft:${activeDraft.id}`)
    return chatStorageKey(user.id, "draft:empty")
  }, [user, workspace, activeDraft])

  const [tools, setTools] = useState<McpTool[] | null>(null)
  const [toolsError, setToolsError] = useState<string | null>(null)
  const [submitting, setSubmitting] = useState(false)
  const [prompt, setPrompt] = useState("")
  // Banner is dismissible per session (spec #398). Sessionstorage so
  // re-mounts/page navs within the same tab don't repop it. Wrapped in
  // try/catch — private-browsing / blocked-storage modes throw on access
  // and we don't want ChatPage to crash because of a chrome detail.
  const [ossBannerDismissed, setOssBannerDismissed] = useState(() => {
    if (typeof window === "undefined") return false
    try {
      return (
        window.sessionStorage.getItem("onsager.oss_banner_dismissed") === "1"
      )
    } catch {
      return false
    }
  })
  const feedEndRef = useRef<HTMLDivElement>(null)

  const [turns, setTurns] = useState<ChatTurn[]>(() => {
    if (!user || !storageKey) return []
    return hydrateTurns(loadStoredTurns(storageKey))
  })

  const turnsRef = useRef<ChatTurn[]>(turns)
  useEffect(() => {
    turnsRef.current = turns
  }, [turns])

  const prevStorageKeyRef = useRef(storageKey)
  useEffect(() => {
    if (storageKey === prevStorageKeyRef.current) return
    prevStorageKeyRef.current = storageKey
    setTurns(storageKey ? hydrateTurns(loadStoredTurns(storageKey)) : [])
  }, [storageKey])

  useEffect(() => {
    if (!storageKey) return
    saveStoredTurns(storageKey, dehydrateTurns(turns))
  }, [turns, storageKey])

  useEffect(() => {
    let cancelled = false
    mcpListTools()
      .then((t) => { if (!cancelled) setTools(t) })
      .catch((e: unknown) => { if (!cancelled) setToolsError(errMsg(e)) })
    return () => { cancelled = true }
  }, [])

  useEffect(() => {
    feedEndRef.current?.scrollIntoView({ behavior: "smooth" })
  }, [turns])

  const llmMessages = useMemo<LlmTurnMessage[]>(() => {
    const out: LlmTurnMessage[] = []
    // Only inject workspace context when one is actually in scope. FTUE
    // workspace-less turns leave this out entirely — the FTUE preamble
    // in the system prompt steers the agent to `propose_workflow_draft`.
    if (workspace) {
      out.push({
        role: "user",
        content: `(context) workspace_id = ${workspace.id}`,
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
  }, [turns, workspace])

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
        // Always pass the resolved workspace id when one exists — even on
        // the FTUE /chat path the relay needs *some* workspace to find an
        // Anthropic credential. The FTUE preamble is what steers tool
        // selection toward the workspace-less draft path.
        workspaceId: workspace?.id,
        ftue: isFtue,
      })
      const assistantMessage: ChatMessage | undefined = result.text
        ? { id: `${turnId}-a`, role: "assistant", content: result.text }
        : undefined
      const newToolCalls: ToolCallEntry[] = result.toolCalls.map((tc) => {
        const isMutation = isMutationTool(tc.name)
        const card = isMutation ? buildCardFor(tc.name, tc.input) : undefined
        const state: HitlCardState = isMutation
          ? card
            ? "pending"
            : "failed"
          : "committing"
        return {
          id: tc.id,
          turnId,
          binding: findMcpTool(tc.name),
          toolName: tc.name,
          input: tc.input,
          card,
          state,
          errorMessage:
            isMutation && !card ? "No card definition for this tool — cannot review." : undefined,
        }
      })
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
  }, [prompt, submitting, tools, llmMessages, workspace, isFtue, autoRunReadOnly])

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
      const target = findCall(turnsRef.current, turnId, callId)
      if (!target) {
        setTurns((prev) =>
          updateCall(prev, turnId, callId, { state: "failed", errorMessage: "Tool call not found." }),
        )
        return
      }
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
        if (
          target.toolName === "propose_workflow" ||
          target.toolName === "propose_workflow_draft"
        ) {
          const doc = extractWorkflowDocument(mergedArgs)
          if (doc) setWorkflow(doc)
        }
      } catch (err) {
        setTurns((prev) =>
          updateCall(prev, turnId, callId, { state: "failed", errorMessage: errMsg(err) }),
        )
      }
    },
    [setWorkflow],
  )

  const handleReject = useCallback((turnId: string, callId: string) => {
    setTurns((prev) => updateCall(prev, turnId, callId, { state: "rejected" }))
  }, [])

  const isEmpty = turns.length === 0

  // Pick a template (spec #406): create a fresh draft seeded with the
  // template's document, record source + template_id on the outer draft
  // (the spec #404 instrumentation hook), and pre-fill the composer with
  // the template's intent so the agent's first reply has context.
  const handlePickTemplate = useCallback(
    (template: FtueTemplate) => {
      const doc = templateToDocument(template)
      newDraft("template", doc, template.name, template.id)
      setPrompt(`Customize "${template.name}" for my project. ${template.intent}`)
    },
    [newDraft],
  )

  // The "describe one yourself" chips are a separate authoring path from
  // the template gallery. If the active draft was seeded from a template,
  // open a fresh blank draft so the right-panel DAG preview doesn't keep
  // showing a stale template shape that no longer matches the prompt.
  const handleChipPick = useCallback(
    (text: string) => {
      setPrompt(text)
      if (activeDraft?.source === "template") {
        newDraft()
      }
    },
    [activeDraft?.source, newDraft],
  )

  return (
    <div className="grid h-full grid-cols-1 md:grid-cols-[minmax(0,2fr)_minmax(0,3fr)]">
      {/* ── Left panel: chat ────────────────────────────────────────── */}
      <div className="flex h-full flex-col overflow-hidden border-r">
        {/* OSS banner (spec #398) — only when running OSS, only on the
            FTUE workspace-less entry, dismissible per session. */}
        {isOss && isFtue && !ossBannerDismissed ? (
          <div className="flex shrink-0 items-center gap-2 border-b bg-muted/40 px-4 py-2 text-xs">
            <span className="text-muted-foreground">
              Running Onsager OSS at localhost. Drafts are stored on this machine.
            </span>
            <a
              href="https://app.onsager.ai"
              target="_blank"
              rel="noreferrer"
              className="ml-auto font-medium text-primary hover:underline"
            >
              Sign in to sync drafts to the cloud →
            </a>
            <Button
              type="button"
              size="icon"
              variant="ghost"
              className="h-6 w-6 text-muted-foreground"
              onClick={() => {
                setOssBannerDismissed(true)
                if (typeof window !== "undefined") {
                  try {
                    window.sessionStorage.setItem(
                      "onsager.oss_banner_dismissed",
                      "1",
                    )
                  } catch {
                    // Private browsing / quota — UI state is the
                    // floor; banner just repops next mount.
                  }
                }
              }}
              aria-label="Dismiss OSS banner"
            >
              <span aria-hidden="true">×</span>
            </Button>
          </div>
        ) : null}
        {/* Drafts quick-access strip (spec #401). Only shown when the
            user has at least one persisted draft. */}
        <DraftStrip
          drafts={drafts}
          activeId={activeDraft?.id ?? null}
          onSwitch={switchDraft}
          onNew={() => newDraft()}
          onDelete={deleteById}
        />
        <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain px-4 py-4">
          {isEmpty ? (
            <EmptyState
              onChip={handleChipPick}
              onPickTemplate={handlePickTemplate}
              selectedTemplateId={activeDraft?.template_id}
              showTemplateGallery={isFtue}
            />
          ) : (
            <ConversationFeed
              turns={turns}
              handleCommit={handleCommit}
              handleReject={handleReject}
              feedEndRef={feedEndRef}
            />
          )}
        </div>

        <div className="shrink-0 border-t bg-background/95 px-4 py-3 backdrop-blur supports-[backdrop-filter]:bg-background/80">
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
                  ? "Describe a workflow, or pick an example above…"
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

      {/* ── Right panel: workflow DAG preview (desktop only) ────────── */}
      <div className="hidden md:block">
        <WorkflowDAGPreview
          draft={workflowDoc}
          onChange={setWorkflow}
          status={dagHeaderStatus(activeDraft)}
        />
      </div>
    </div>
  )
}

// Build the header status line for the right-panel preview per spec
// #401's locked copy: `Draft · <name> · Saved locally` while unbound,
// `Bound · <name> · Open in Workflows →` once the binding flow finishes.
function dagHeaderStatus(draft: WorkflowDraft | null): string | undefined {
  if (!draft) return undefined
  const name = draft.name || draft.workflow.name || "Untitled workflow"
  if (draft.bound_to) return `Bound · ${name} · Open in Workflows →`
  return `Draft · ${name} · Saved locally`
}

// ─── Empty state ─────────────────────────────────────────────────────────────

// Workspace-free example chips per spec #400 — they teach the user the
// shape of *questions* to ask (the gallery teaches the shape of
// outputs). Locked copy.
const EXAMPLE_CHIPS = [
  "Walk me through the auto-merge-on-green template.",
  "What's a Verify gate? Show me one.",
  "Draft a workflow that summarizes labeled issues.",
]

interface EmptyStateProps {
  onChip: (text: string) => void
  onPickTemplate: (template: FtueTemplate) => void
  selectedTemplateId?: string
  showTemplateGallery: boolean
}

function EmptyState({
  onChip,
  onPickTemplate,
  selectedTemplateId,
  showTemplateGallery,
}: EmptyStateProps) {
  return (
    <div className="flex flex-col items-center gap-6 py-12 text-center">
      <div className="flex h-14 w-14 items-center justify-center rounded-full bg-primary/10 text-primary">
        <MessageSquare className="h-7 w-7" />
      </div>
      <div className="flex flex-col gap-2">
        <h2 className="text-2xl font-bold tracking-tight">Design something.</h2>
        <p className="max-w-sm text-sm text-muted-foreground">
          Describe what you want to automate. I&apos;ll propose a workflow, you
          review it, and one click ships it.
        </p>
      </div>
      {showTemplateGallery && (
        <div className="w-full max-w-3xl px-2">
          <TemplateGallery onPick={onPickTemplate} selectedId={selectedTemplateId} />
        </div>
      )}
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
      {/* Spec #408 location 1: factory-metaphor copy on the chat empty state. */}
      <p className="max-w-md text-xs text-muted-foreground">
        <Sparkles className="mr-1 inline h-3 w-3" />
        Cards along the way are inspection reports — what&apos;s about to
        change at each QC checkpoint. Nothing ships until you accept.
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
    <div className="flex flex-col gap-4 pb-4">
      {turns.map((turn) => (
        <div key={turn.id} className="flex flex-col gap-2">
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
        </div>
      ))}
      <div ref={feedEndRef} />
    </div>
  )
}

// ─── Bubbles and blocks ───────────────────────────────────────────────────────

function UserBubble({ content }: { content: string }) {
  return (
    <div className="self-end max-w-[85%] rounded-2xl rounded-tr-sm bg-primary px-3 py-2 text-sm text-primary-foreground">
      {content}
    </div>
  )
}

function AssistantBubble({ content }: { content: string }) {
  return (
    <div className="max-w-[90%] rounded-2xl rounded-tl-sm bg-muted/40 px-3 py-2 text-sm">
      <Markdown
        rehypePlugins={[rehypeHighlight]}
        components={{
          pre: ({ children }) => (
            <pre className="my-1 max-h-64 overflow-auto rounded-md border bg-background/60 p-2 text-[11px]">
              {children}
            </pre>
          ),
          code: ({ children, className }) =>
            className ? (
              <code className={className}>{children}</code>
            ) : (
              <code className="rounded bg-muted px-1 py-0.5 font-mono text-[11px]">{children}</code>
            ),
          p: ({ children }) => <p className="mb-1 last:mb-0">{children}</p>,
        }}
      >
        {content}
      </Markdown>
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
