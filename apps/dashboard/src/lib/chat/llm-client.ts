import Anthropic from "@anthropic-ai/sdk"
import type { McpTool } from "@/lib/mcp-client"

// Latest Claude model id per the claude-api skill (Opus 4.7). The
// dashboard chat runs same-origin against `/mcp/messages` for tool
// execution; the LLM call goes direct to Anthropic from the browser
// (with `dangerouslyAllowBrowser`) using a key the user wires into
// the dashboard env. This is acceptable pre-launch — the only humans
// holding keys are the team building Onsager. Post-launch the call
// moves behind a portal-hosted relay (a follow-up).
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
  /** Optional override; defaults to `VITE_ANTHROPIC_API_KEY`. */
  apiKey?: string
  /** Optional model override; defaults to the constant above. */
  model?: string
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
 * Run one LLM turn. Returns the assistant text + any tool_use blocks.
 * Prompt caching is applied to the system prompt and tool definitions
 * (they are stable across turns within a session); message history
 * is not cached for v1.
 */
export async function runChatTurn(args: RunChatArgs): Promise<LlmTurn> {
  const apiKey = args.apiKey ?? readBrowserEnv("VITE_ANTHROPIC_API_KEY")
  if (!apiKey) {
    throw new LlmConfigError(
      "Anthropic API key missing. Set VITE_ANTHROPIC_API_KEY in the " +
        "dashboard env, or pass an explicit `apiKey` to the chat client.",
    )
  }
  const client = new Anthropic({ apiKey, dangerouslyAllowBrowser: true })

  const tools = args.tools.map((t) => ({
    name: t.name,
    description: t.description,
    input_schema: t.inputSchema as Anthropic.Tool.InputSchema,
  }))

  const resp = await client.messages.create({
    model: args.model ?? DEFAULT_MODEL,
    max_tokens: MAX_TOKENS,
    system: [
      {
        type: "text",
        text: SYSTEM_PROMPT,
        cache_control: { type: "ephemeral" },
      },
    ],
    // `tools` shares the same cache block as the system prompt: when
    // the tool list is unchanged across turns Anthropic returns a cache
    // hit on the prefix. The cache_control marker on the last system
    // block instructs the server to cache everything before the first
    // user message in this request.
    tools,
    messages: args.messages.map((m) => ({
      role: m.role,
      content: m.content,
    })),
  })

  let text = ""
  const toolCalls: LlmToolCall[] = []
  for (const block of resp.content) {
    if (block.type === "text") {
      text += block.text
    } else if (block.type === "tool_use") {
      toolCalls.push({
        id: block.id,
        name: block.name,
        input: (block.input ?? {}) as Record<string, unknown>,
      })
    }
  }
  return { text, toolCalls }
}

function readBrowserEnv(name: string): string | undefined {
  // Vite exposes env vars under `import.meta.env`; the cast keeps
  // tsc happy without a global ambient declaration.
  const env = (import.meta as unknown as { env?: Record<string, string> }).env
  return env?.[name]
}
