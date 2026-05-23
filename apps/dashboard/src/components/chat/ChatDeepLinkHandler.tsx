import { useEffect } from "react"
import { useLocation, useNavigate } from "react-router-dom"
import { useChatUi } from "@/lib/chat/use-chat-ui"
import { parseScopeKey, WORKSPACE_SCOPE } from "@/lib/chat/scope"

// ChatDeepLinkHandler (ADR 0020 § Invocation channels, spec #472).
//
// Push notification clicks land the user at an in-app URL that carries
// the HITL the push was about. The service worker shapes the URL as:
//
//   {workspaceUrl}?onsager_chat=open&onsager_chat_scope=<scope-key>
//                 &onsager_hitl_id=<id>
//
// This component watches the URL on mount + on every location change,
// opens the global chat at the requested scope with the Queue tab
// highlighted, and strips the params so a refresh doesn't re-open. The
// Queue card highlight is driven by `useChatUi.highlightHitlId`.

const PARAM_OPEN = "onsager_chat"
const PARAM_SCOPE = "onsager_chat_scope"
const PARAM_HITL = "onsager_hitl_id"

export function ChatDeepLinkHandler() {
  const { open } = useChatUi()
  const location = useLocation()
  const navigate = useNavigate()

  useEffect(() => {
    const params = new URLSearchParams(location.search)
    if (params.get(PARAM_OPEN) !== "open") return

    const scopeKey = params.get(PARAM_SCOPE)
    const hitlId = params.get(PARAM_HITL)
    const scope = scopeKey ? (parseScopeKey(scopeKey) ?? WORKSPACE_SCOPE) : WORKSPACE_SCOPE

    open({
      scope,
      tab: "queue",
      highlightHitlId: hitlId ?? undefined,
    })

    // Strip the params so a refresh / back doesn't re-trigger.
    params.delete(PARAM_OPEN)
    params.delete(PARAM_SCOPE)
    params.delete(PARAM_HITL)
    const remaining = params.toString()
    navigate(
      {
        pathname: location.pathname,
        search: remaining ? `?${remaining}` : "",
        hash: location.hash,
      },
      { replace: true },
    )
  }, [location.search, location.pathname, location.hash, open, navigate])

  return null
}
