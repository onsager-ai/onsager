import { streamText, jsonSchema, tool } from "ai"
import { createAnthropic } from "@ai-sdk/anthropic"
import type { McpTool } from "@/lib/mcp-client"

// Latest Claude model per the claude-api skill (Opus 4.7). API key is
// read from localStorage at runtime — never from import.meta.env.VITE_*,
// which Vite inlines into the bundle. The proper end-state is a
// portal-hosted relay; filed as a follow-up.
const DEFAULT_MODEL = "claude-opus-4-7"
const MAX_TOKENS = 8192
const API_KEY_STORAGE_KEY = "onsager.anthropic.apiKey"

export interface LlmTurn {
  text: string
  toolCalls: LlmToolCall[]
}

export interface LlmToolCall {
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
  apiKey?: string
  model?: string
  /** Called with each streamed text delta as it arrives. */
  onTextChunk?: (chunk: string) => void
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

/**
 * Run one LLM turn with streaming. Calls `onTextChunk` for each text
 * delta as it arrives; resolves with the full text and any tool calls
 * once the stream closes.
 *
 * Uses Vercel AI SDK + @ai-sdk/anthropic. MCP tool schemas are passed
 * as raw JSON Schema via the `jsonSchema()` helper — no Zod conversion.
 * Prompt caching is delegated to the Anthropic provider's default
 * behaviour on the system prompt block.
 */
export async function runChatTurn(args: RunChatArgs): Promise<LlmTurn> {
  const apiKey = args.apiKey ?? readApiKey()
  if (!apiKey) {
    throw new LlmConfigError(
      `Anthropic API key missing. Set it in your browser via ` +
        `\`localStorage.setItem("${API_KEY_STORAGE_KEY}", "sk-ant-…")\` ` +
        "and reload, or pass an explicit `apiKey` to the chat client. " +
        "(The key is never read from build-time env vars.)",
    )
  }

  const anthropic = createAnthropic({ apiKey })

  // Convert MCP JSON Schema tool definitions. Tools without an `execute`
  // function are returned to the caller as tool-call objects; the AI SDK
  // does not auto-execute them.
  const tools = Object.fromEntries(
    args.tools.map((t) => [
      t.name,
      tool({
        description: t.description,
        parameters: jsonSchema(t.inputSchema as Parameters<typeof jsonSchema>[0]),
      }),
    ]),
  )

  const result = streamText({
    model: anthropic(args.model ?? DEFAULT_MODEL),
    system: SYSTEM_PROMPT,
    messages: args.messages.map((m) => ({ role: m.role, content: m.content })),
    tools,
    maxTokens: MAX_TOKENS,
  })

  let text = ""
  for await (const chunk of result.textStream) {
    text += chunk
    args.onTextChunk?.(chunk)
  }

  const toolCalls = await result.toolCalls

  return {
    text,
    toolCalls: (toolCalls ?? []).map((tc) => ({
      id: tc.toolCallId,
      name: tc.toolName,
      input: (tc.args ?? {}) as Record<string, unknown>,
    })),
  }
}

function readApiKey(): string | undefined {
  if (typeof window === "undefined" || !window.localStorage) return undefined
  const v = window.localStorage.getItem(API_KEY_STORAGE_KEY)
  return v && v.trim() !== "" ? v : undefined
}
