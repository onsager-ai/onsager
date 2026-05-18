import { API_BASE, ApiError } from "@/lib/api/client"
import type { McpTool } from "@/lib/mcp-client"

// Latest Claude model id per the claude-api skill (Opus 4.7).
const DEFAULT_MODEL = "claude-opus-4-7"
const MAX_TOKENS = 1024

/**
 * One chat turn from the LLM. `text` is rendered as plain markdown;
 * `toolCalls` each route through the HitlCard (mutations) or info-block
 * (read-only) path.
 */
export interface LlmTurn {
  text: string
  toolCalls: LlmToolCall[]
}

export interface LlmToolCall {
  /** Anthropic-side tool_use id; we round-trip it on tool_result. */
  id: string
  name: string
  input: Record<string, unknown>
}

export interface LlmTurnMessage {
  role: "user" | "assistant"
  content: string
}

export interface RunChatArgs {
  messages: LlmTurnMessage[]
  tools: McpTool[]
  workspaceId?: string
  /** Optional model override; defaults to the constant above. */
  model?: string
  /**
   * Prepend the FTUE preamble (spec #400) to the system prompt. Set to
   * `true` when chatting from the workspace-less `/chat` entry so the
   * agent knows to propose drafts rather than ask for workspace/repo
   * context.
   */
  ftue?: boolean
}

export class LlmConfigError extends Error {}

const SYSTEM_PROMPT = [
  "You are the Onsager workflow assistant — an AI factory operator embedded",
  "in the Onsager dashboard. You help humans design, run, and triage",
  "AI-driven workflows. You speak through MCP tools the dashboard hosts.",
  "",
  "Rules:",
  "- Prefer tools over prose for any state mutation. Never describe a",
  "  change without proposing it as a tool call the user can edit and",
  "  commit via the HITL card.",
  "- Read-only tools (list_*, inspect_*, get_*) render as plain info",
  "  blocks; mutation tools (propose_*, run_*, edit_*, schedule_*,",
  "  cancel_*) render as HitlCards the user reviews.",
  "- Onsager's user-facing vocabulary is exactly Workflow, Run, Artifact,",
  "  Stage. Use those nouns in copy; never `bundle`, `sealed`, or `spec`.",
  "- If a tool call needs a workspace_id and one was not provided in the",
  "  conversation, ask before guessing.",
].join("\n")

// Locked preamble (per spec #400) prepended to the system prompt when the
// user is in FTUE workspace-less mode. Iterating the wording is a
// follow-up — touch this only with an accompanying spec amendment.
const FTUE_PREAMBLE = [
  "You are helping a new Onsager user design their first workflow draft.",
  "They do not have a workspace yet. Drafts live client-side until the user",
  "chooses to bind. Do not ask for a workspace, repo, or installation —",
  "those are for the binding step. If a tool requires workspace context,",
  'prefer "propose_workflow_draft" (no workspace_id) over "create_workflow".',
  "Be concrete: end your first reply with a workflow draft proposal.",
].join("\n")

/** Wire shape of one content block in the Anthropic response. */
interface AnthropicContentBlock {
  type: string
  text?: string
  id?: string
  name?: string
  input?: Record<string, unknown>
}

/** Minimal Anthropic Messages API response shape the relay returns. */
interface AnthropicResponse {
  content: AnthropicContentBlock[]
}

/**
 * Run one LLM turn via the portal relay at `/api/chat/completions`.
 * The API key never touches the browser — portal resolves it from the
 * workspace `anthropic` credential (spec #318).
 *
 * Prompt caching is applied to the system prompt via `cache_control`;
 * the relay injects the `anthropic-beta: prompt-caching-2024-07-31`
 * header server-side.
 */
export async function runChatTurn(args: RunChatArgs): Promise<LlmTurn> {
  // Workspace-less FTUE callers (spec #398 `/chat` entry) pass no
  // workspace id. The relay still needs *some* workspace to resolve an
  // anthropic credential against; the dashboard injects the user's
  // last-used or memberships[0] when available. If neither is set, the
  // user truly has zero workspaces and we surface a typed error the
  // ChatPage renders as an inline "set up your Anthropic credential"
  // prompt instead of swallowing into a generic 4xx.
  if (!args.workspaceId) {
    throw new LlmConfigError(
      "Onsager needs a workspace to reach Anthropic. Create one to chat with the agent.",
    )
  }

  const tools = args.tools.map((t) => ({
    name: t.name,
    description: t.description,
    input_schema: t.inputSchema,
  }))

  // FTUE preamble is locked text per spec #400. Cached separately from
  // the canonical system prompt so the Anthropic prompt-cache hit rate
  // stays good across workspace and FTUE turns alike.
  const systemBlocks: Array<{
    type: "text"
    text: string
    cache_control?: { type: "ephemeral" }
  }> = [
    { type: "text", text: SYSTEM_PROMPT, cache_control: { type: "ephemeral" } },
  ]
  if (args.ftue) {
    systemBlocks.push({
      type: "text",
      text: FTUE_PREAMBLE,
      cache_control: { type: "ephemeral" },
    })
  }

  const res = await fetch(`${API_BASE}/chat/completions`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      workspace_id: args.workspaceId,
      model: args.model ?? DEFAULT_MODEL,
      max_tokens: MAX_TOKENS,
      system: systemBlocks,
      tools,
      messages: args.messages.map((m) => ({
        role: m.role,
        content: m.content,
      })),
    }),
  })

  if (!res.ok) {
    const err = await res.json().catch(() => ({ error: res.statusText }))
    if (res.status === 422 && err.error === "anthropic_credential_missing") {
      throw new LlmConfigError(
        "Anthropic credential not set. Add an `anthropic` credential in " +
          "workspace Settings → Credentials.",
      )
    }
    throw new ApiError(err.detail || err.error || res.statusText, res.status)
  }

  const resp: AnthropicResponse = await res.json()

  let text = ""
  const toolCalls: LlmToolCall[] = []
  for (const block of resp.content) {
    if (block.type === "text" && block.text != null) {
      text += block.text
    } else if (block.type === "tool_use" && block.id && block.name) {
      toolCalls.push({
        id: block.id,
        name: block.name,
        input: (block.input ?? {}) as Record<string, unknown>,
      })
    }
  }
  return { text, toolCalls }
}
