import {
  type FormEvent,
  type KeyboardEvent,
  useCallback,
  useEffect,
  useRef,
  useState,
} from "react"
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { AlertTriangle, MessageSquare, Send } from "lucide-react"
import Markdown from "react-markdown"
import { Button } from "@/components/ui/button"
import { Textarea } from "@/components/ui/textarea"
import type { Workspace } from "@/lib/api"
import { chat, type ChatScopeWire } from "@/lib/api/chat"
import { LlmConfigError, runChatTurn } from "@/lib/chat/llm-client"
import type { ChatMessage } from "@/lib/api/generated/ChatMessage"
import type { ChatScope } from "@/lib/chat/scope"
import { scopeKey } from "@/lib/chat/scope"
import { PreviousConversationsLink } from "./PreviousConversationsLink"

// Thread tab — the rolling conversation for the active scope.
//
// Loads messages from `GET /api/chat/thread` (spec #470); the send
// path appends the user message, runs an LLM turn through the
// portal relay (`/api/chat/completions`, no tools in this phase),
// then appends the assistant text. The full MCP tool-call + HITL
// surface is folded back in by #472 (it owns the invocation channels
// and the action-button binding); this phase ships the storage-
// backed thread skeleton.

interface ChatThreadViewProps {
  workspace: Workspace
  scope: ChatScope
  userId: string | null
  /** One-shot composer pre-fill from `useChatUi.open({ prefill })` (#473). */
  initialPrompt?: string | null
}

function toWireScope(scope: ChatScope): ChatScopeWire {
  if (scope.type === "workspace") return { scope_type: "workspace" }
  if (!scope.id) {
    // shouldn't happen — the auto-scope hook always pairs non-
    // workspace types with an id. Fall back to workspace so the
    // request shape stays valid.
    return { scope_type: "workspace" }
  }
  return { scope_type: scope.type, scope_id: scope.id }
}

export function ChatThreadView({
  workspace,
  scope,
  userId,
  initialPrompt,
}: ChatThreadViewProps) {
  const qc = useQueryClient()
  const threadKey = ["chat", "thread", workspace.id, scopeKey(scope)]
  const threadQuery = useQuery({
    queryKey: threadKey,
    queryFn: () => chat.getThread(workspace.id, toWireScope(scope)),
    // Conversations rarely change underneath the user; refetch
    // explicitly after a send rather than on every focus.
    staleTime: 60_000,
  })

  const [prompt, setPrompt] = useState(initialPrompt ?? "")
  const [sendError, setSendError] = useState<string | null>(null)
  const feedEndRef = useRef<HTMLDivElement>(null)
  const textareaRef = useRef<HTMLTextAreaElement>(null)

  useEffect(() => {
    feedEndRef.current?.scrollIntoView({ behavior: "smooth" })
  }, [threadQuery.data?.messages])

  // When the chat opens with a pre-fill (⌘K → `ask: …`, #473), drop
  // the caret at the end of the seeded text so Enter sends immediately
  // or the user can keep typing. Mount-only: the parent (ChatContainer)
  // unmounts on close, so each open() with a fresh prefill remounts
  // this view — re-running on every initialPrompt change would steal
  // focus if the prop pointer ever flipped mid-session.
  useEffect(() => {
    if (initialPrompt) {
      const el = textareaRef.current
      if (el) {
        el.focus()
        const len = initialPrompt.length
        el.setSelectionRange(len, len)
      }
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const sendMutation = useMutation({
    mutationFn: async (text: string) => {
      const threadId = threadQuery.data?.thread.id
      if (!threadId) throw new Error("Chat thread not loaded yet")

      // Append the user message immediately so the UI feels live;
      // we'll refetch after the assistant reply lands.
      await chat.appendMessage(threadId, workspace.id, "user", text)

      // Build the LLM-side conversation from the loaded thread plus
      // the new turn. No tools attached in this phase — the MCP
      // wiring + HITL routing folds back in via #472.
      const priorMessages = (threadQuery.data?.messages ?? []).flatMap((m) =>
        m.role === "user" || m.role === "assistant"
          ? [{ role: m.role as "user" | "assistant", content: m.content }]
          : [],
      )
      const turn = await runChatTurn({
        messages: [...priorMessages, { role: "user", content: text }],
        tools: [],
        workspaceId: workspace.id,
      })

      if (turn.text) {
        await chat.appendMessage(threadId, workspace.id, "assistant", turn.text)
      }
      return turn
    },
    onError: (err: unknown) => {
      const msg =
        err instanceof LlmConfigError
          ? err.message
          : err instanceof Error
            ? err.message
            : String(err)
      setSendError(msg)
    },
    onSuccess: () => {
      setSendError(null)
      qc.invalidateQueries({ queryKey: threadKey })
    },
  })

  const doSubmit = useCallback(() => {
    const text = prompt.trim()
    if (!text || sendMutation.isPending) return
    setPrompt("")
    sendMutation.mutate(text)
  }, [prompt, sendMutation])

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

  const messages = threadQuery.data?.messages ?? []
  const isWorkspaceScope = scope.type === "workspace"

  return (
    <div
      data-slot="chat-thread"
      data-scope={scope.type}
      className="flex min-h-0 flex-1 flex-col"
    >
      <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain px-3 py-3">
        {isWorkspaceScope ? (
          <PreviousConversationsLink userId={userId} />
        ) : null}
        {threadQuery.isLoading ? (
          <LoadingState />
        ) : threadQuery.isError ? (
          <ErrorState message={(threadQuery.error as Error).message} />
        ) : messages.length === 0 ? (
          <EmptyState />
        ) : (
          <MessageList messages={messages} feedEndRef={feedEndRef} />
        )}
        {sendMutation.isPending ? (
          <div className="mt-2 text-xs italic text-muted-foreground">
            Thinking…
          </div>
        ) : null}
        {sendError ? (
          <div
            role="alert"
            className="mt-2 flex items-start gap-2 rounded-md border border-destructive/40 bg-destructive/5 p-2 text-xs text-destructive"
          >
            <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
            <span>{sendError}</span>
          </div>
        ) : null}
      </div>

      <form
        onSubmit={onFormSubmit}
        className="flex shrink-0 items-end gap-2 border-t bg-background/95 p-2 backdrop-blur supports-[backdrop-filter]:bg-background/80"
      >
        <Textarea
          ref={textareaRef}
          value={prompt}
          onChange={(e) => setPrompt(e.target.value)}
          onKeyDown={onKeyDown}
          placeholder="Ask…"
          rows={2}
          disabled={threadQuery.isLoading || sendMutation.isPending}
          className="flex-1 resize-none text-sm"
          aria-label="Chat message"
        />
        <Button
          type="submit"
          size="sm"
          disabled={
            !prompt.trim() ||
            threadQuery.isLoading ||
            sendMutation.isPending
          }
          aria-label="Send message"
        >
          <Send className="h-4 w-4" />
        </Button>
      </form>
    </div>
  )
}

function LoadingState() {
  return (
    <div className="flex flex-col gap-2 pb-2">
      <div className="h-6 w-32 animate-pulse rounded bg-muted/60" />
      <div className="h-12 w-3/4 animate-pulse rounded bg-muted/60" />
      <div className="h-6 w-40 animate-pulse rounded bg-muted/60" />
    </div>
  )
}

function ErrorState({ message }: { message: string }) {
  return (
    <div
      role="alert"
      className="flex items-start gap-2 rounded-md border border-destructive/40 bg-destructive/5 p-2 text-xs text-destructive"
    >
      <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
      <span>Could not load thread — {message}</span>
    </div>
  )
}

function EmptyState() {
  return (
    <div
      data-slot="chat-thread-empty"
      className="flex flex-col items-center gap-2 py-8 text-center text-sm text-muted-foreground"
    >
      <MessageSquare className="h-8 w-8 text-muted-foreground/50" />
      <div>Start the conversation for this scope.</div>
      <div
        data-slot="chat-thread-empty-hint"
        className="hidden text-xs text-muted-foreground/80 md:block"
      >
        Tip — press{" "}
        <kbd className="rounded border bg-muted px-1.5 py-0.5 font-mono text-[10px]">
          ⌘K
        </kbd>{" "}
        then type{" "}
        <code className="rounded bg-muted px-1 py-0.5 font-mono text-[11px]">
          ask:
        </code>{" "}
        followed by your question.
      </div>
    </div>
  )
}

function MessageList({
  messages,
  feedEndRef,
}: {
  messages: ChatMessage[]
  feedEndRef: React.RefObject<HTMLDivElement | null>
}) {
  return (
    <div className="flex flex-col gap-3">
      {messages.map((m) =>
        m.role === "user" ? (
          <UserBubble key={m.id} content={m.content} />
        ) : m.role === "assistant" ? (
          <AssistantBubble key={m.id} content={m.content} />
        ) : null,
      )}
      <div ref={feedEndRef} />
    </div>
  )
}

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
        components={{
          p: ({ children }) => <p className="mb-1 last:mb-0">{children}</p>,
          code: ({ children, className }) =>
            className ? (
              <code className={className}>{children}</code>
            ) : (
              <code className="rounded bg-muted px-1 py-0.5 font-mono text-[11px]">
                {children}
              </code>
            ),
        }}
      >
        {content}
      </Markdown>
    </div>
  )
}
