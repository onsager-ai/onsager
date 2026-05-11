// Same-origin MCP client. Speaks JSON-RPC 2.0 against
// `POST /mcp/messages` (transport defined by spec #288 / ADR 0007).
//
// Two surfaces matter for the dashboard chat:
//
// 1. `tools/list` — the registry the LLM is given as available tools.
// 2. `tools/call` — invoked when the user commits a HitlCard.
//
// All calls go through the same fetch path; cookies (session auth)
// ride along same-origin. No PATs in browser code.

export const MCP_PATH = "/mcp/messages"

export interface McpTool {
  name: string
  description: string
  inputSchema: Record<string, unknown>
}

export interface McpToolCallResult {
  content: { type: string; text?: string }[]
  structuredContent?: unknown
  isError?: boolean
}

export class McpClientError extends Error {
  code: number
  constructor(message: string, code: number) {
    super(message)
    this.code = code
  }
}

interface JsonRpcResponse<T> {
  jsonrpc: "2.0"
  id: number | string | null
  result?: T
  error?: { code: number; message: string; data?: unknown }
}

let nextId = 1

async function rpc<T>(method: string, params: unknown = {}): Promise<T> {
  const id = nextId++
  const res = await fetch(MCP_PATH, {
    method: "POST",
    credentials: "same-origin",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ jsonrpc: "2.0", id, method, params }),
  })
  if (!res.ok) {
    throw new McpClientError(
      `MCP HTTP error: ${res.status} ${res.statusText}`,
      res.status,
    )
  }
  const body = (await res.json()) as JsonRpcResponse<T>
  if (body.error) {
    throw new McpClientError(body.error.message, body.error.code)
  }
  if (body.result === undefined) {
    throw new McpClientError("MCP response missing `result`", -32000)
  }
  return body.result
}

export async function mcpListTools(): Promise<McpTool[]> {
  const r = await rpc<{ tools: McpTool[] }>("tools/list")
  return r.tools
}

export async function mcpCallTool(
  name: string,
  args: Record<string, unknown>,
): Promise<McpToolCallResult> {
  return rpc<McpToolCallResult>("tools/call", { name, arguments: args })
}
