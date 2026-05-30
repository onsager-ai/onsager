# ADR 0020 — Chat as portable global component with contextual auto-scope

- **Status**: Accepted
- **Date**: 2026-05-22
- **Identity impact**: no
- **Tracking issues**: #451 (implementation spec). Sibling to ADR 0019, which removes the top-level Chat tab. This ADR governs what chat *becomes* once it is no longer a tab. Builds on ADR 0007 (tools and skills as the public contract) and ADR 0008 (portal owns the agent control plane).
- **Supersedes**: none. Predecessor specs (#398 universal-chat-entry, #400 chat empty-state chips, #401 DraftStrip + chat surface) are the prior frame in prose; the repo convention reserves the `Supersedes` field for ADR-to-ADR replacement, so no entry here.
- **Superseded by**: none
- **Amended by**: ADR 0025 (2026-05-29) — names chat's dashboard role as design + maintenance (not a pure interchangeable frontend) and adds a labeled desktop entry. The responsive-mount, auto-scope *mechanism* (the hook, pin override, storage schema) is unchanged; the route → default-scope *table* below is narrowed to design (workspace) + maintenance (run / verdict) by ADR 0025 / spec #514 — a workflow detail route no longer auto-scopes to `workflow:{id}` (that scope survives only as an explicit Ask override).

## Context

The dashboard’s chat surface is a page today — `ChatPage.tsx` (1005 lines), reachable via two routes (`/chat` and `/workspaces/:slug/chat`). It hosts a conversation feed with HITL cards, a draft chip strip, a right-pane DAG preview, an empty-state template gallery, and a stack of banners (OSS notice, webhook health warning). The right-pane DAG preview is rendered as a 60% viewport strip even when no draft is active.

This commits the chat to one of the four top-level dashboard surfaces that spec #289’s two-surface IA recognized. With ADR 0019 collapsing the IA to three tabs (Workflows / Runs / Activity) and removing Chat from top-level, the question becomes: how does a user reach chat, and in what form?

KB position (Notion `35cd79199ca68131b4f6efac9ed8c3d6`, Onsager root note) frames the substrate as UI-portable via the MCP control plane:

> Onsager substrate UI-portable via MCP control plane (intent / spec_plan / compile / run / status / artifacts / verdicts); Claude / Cursor / Onsager UI 互换 intent frontends

ADR 0007 names the same point at the protocol layer: tools and skills are the public contract; the dashboard chat UI is one of N consumers of that contract, not a privileged one. ADR 0008 then carves portal as the owner of the public surface those consumers reach. Together they imply that chat inside the dashboard:

- Is one MCP client among Claude, Cursor, and any other intent frontend
- Should not occupy a privileged top-level position relative to other user activities
- Should be reachable from anywhere a user might form an intent
- Should be aware of what the user is currently looking at, without forcing them to restate it

A second concern is form and entry. The earlier framing of this ADR listed four "mount forms" (FAB, Sidebar, Drawer, Palette) as user- selectable shells. A mobile-first design review found this conflated two orthogonal axes: where the chat surface lives in the viewport (mount form) and how the user opens it (invocation channel). FAB and ⌘K are invocation channels, not viewport forms; Drawer and Sidebar are the same surface positioned differently across mobile and desktop. One responsive mount with several invocation channels fits the P1 ICP (Staff/Principal Engineer working across mobile and desktop) without forcing a "pick your shell" choice users should not have to think about.

The third concern is scope. The current `chatStorageKey(user.id, workspace.id)` at `apps/dashboard/src/pages/ChatPage.tsx:293-297` ties one thread per workspace. Users investigating multiple workflows in the same workspace lose continuity: every conversation about workflow A is mixed into the same thread as conversations about workflow B. A finer scoping is needed.

## Decision

Chat becomes a portable global component: one responsive mount, five invocation channels, one shared state and storage layer.

### Mount: one responsive component

A single `ChatContainer` adapts to the viewport — no user-selectable mount form.

|Platform|Position               |Size                            |Dismiss                  |
|--------|-----------------------|--------------------------------|-------------------------|
|Mobile  |Bottom-anchored overlay|~70% viewport height, full width|Drag-down or close button|
|Desktop |Right-anchored sidebar |420px fixed width, full height  |`⌘J` or close button     |

Both render the same Hybrid surface — a segmented control with two tabs, `Queue` (HITL aggregator for the current scope) and `Thread` (rolling conversation). Default tab is `Queue` when scope has pending HITL, `Thread` otherwise.

### Invocation channels

Five channels open the same mount. Guiding rule: **more prominent entry → more specific scope** — the loud FAB lands in the current route; the quieter top-chrome bell lands in the workspace-wide Inbox. The inversion would surprise users.

|#|Channel                 |Default scope                           |Default tab                  |Platform                            |
|-|------------------------|----------------------------------------|-----------------------------|------------------------------------|
|1|Push notification       |HITL's scope                            |Queue + card highlighted     |Mobile + desktop notification center|
|2|FAB (scope-aware)       |Current route                           |Queue if pending, else Thread|Mobile                              |
|3|Top chrome bell         |`workspace` (Inbox)                     |Queue                        |Mobile + desktop                    |
|4|Scope-local "Ask" button|Object-level (step / verdict / artifact)|Thread                       |Mobile + desktop                    |
|5|⌘K palette              |Current route                           |Thread                       |Desktop                             |

Only push notification is a deep link, carrying an HITL id; the other four land in the mount's default state for the resolved scope. Push originates on the portal MCP control plane (ADR 0008); the dashboard consumes it as one MCP client among many (ADR 0007).

### FAB three-state behavior (mobile)

States are driven by scope HITL state and viewport conditions, not user toggle. Long-press jumps to the Inbox (`scope=workspace`); a first-time hint surfaces on the third FAB use.

|State |Trigger                             |Visual                                                                                               |
|------|------------------------------------|-----------------------------------------------------------------------------------------------------|
|Active|Current scope has pending HITL      |52px filled circle, foreground icon, accent badge with count, 3-second pulse ring on new HITL arrival|
|Muted |Current scope has no pending HITL   |40px outlined circle, secondary icon color, no badge, opacity 0.75                                   |
|Hidden|Scroll-down velocity, or keyboard up|Fade-out + 8px downward translate; restore on scroll-up or keyboard dismiss                          |

### Notification action buttons

- **Approval-kind HITL** ships `Approve` / `Reject` buttons, handled in the notification surface without opening the app. The handler relays the choice to the portal MCP control plane (ADR 0008).
- **Selection-kind and Clarify-kind HITL** ship only `Open`. These kinds need additional input the notification surface cannot capture; `Open` lands on the highlighted Queue card.

The callback → MCP elicitation response binding is a follow-up ADR (see Out of scope).

### Contextual auto-scope

The chat scope follows the user’s current view:

|Current view                            |Default chat scope              |
|----------------------------------------|--------------------------------|
|Workflows tab list                      |`workspace`                     |
|Workflow detail                         |`workflow:{id}`                 |
|Run detail                              |`run:{id}`                      |
|Verdict detail (within run)             |`verdict:{id}`                  |
|Workspace switch                        |scope reset to new workspace    |
|Tab switch (Workflows ↔ Runs ↔ Activity)|scope preserved at current level|

Each scope holds **one rolling thread**. Returning to a previously visited scope resumes the same conversation; no implicit thread creation. A pin control in the chat header locks the scope against navigation changes, supporting cross-context inquiry — “looking at workflow A while asking a question about workflow B” — without forking the conversation model.

Cross-scope history is fully indexed on the backend. The scope filter is a default view, not a wall: when the user asks “what did we decide about release notes last week?” inside a workflow A scope, the assistant searches across all the user’s scopes in the current workspace and cites the source scope alongside the answer.

The chat header renders the scope path as a breadcrumb: `acme-corp / Auto-merge on green / Run #42`. Pin state shows as a filled-icon affordance next to the breadcrumb.

### Storage schema change

```
old: chatStorageKey(user_id, workspace_id) → localStorage entry
new: chatStorageKey(user_id, scope_type, scope_id) → backend (server-side)
```

The old single-thread-per-workspace key collapses to `scope_type='workspace'`; new finer scopes (`workflow`, `run`, `verdict`) get their own keys. The storage backend moves from client-side `localStorage` to server-side persistence; the engine choice (Postgres, spine, dedicated chat store) is delegated to the implementation spec.

**Migration strategy: fresh start.** Onsager is pre-launch; existing chat-storage entries belong to OSS dogfood users only. Their `localStorage` entries are read once at first login after this ADR ships and surfaced as a one-time “previous conversations” link inside the workspace-scoped thread. Subsequent writes go to the backend under the new schema. No automated data migration ships.

### Empty-state behaviour

After ADR 0019 + this ADR land, signed-in users with zero workflows land on the Workflows tab and see an empty state. The empty state displays a “Start in chat →” card that opens the chat surface, scoped to the workspace. The chat empty state itself shows the template gallery from `apps/dashboard/src/lib/templates/v0.json` (per spec #406, content unchanged). The DraftStrip from `apps/dashboard/src/components/chat/ DraftStrip.tsx` is removed; its function (switching between in-flight drafts) is absorbed into the Workflows tab’s state-filter chips when filtered to `Drafted`.

## Rejected alternatives

- **Keep chat as a page (one of the three top-level tabs).** Considered as a “Chat / Workflows / Runs” tab trio. Rejected because it would re-create the spec #289 mode-split that pipeline- centric IA explicitly retires, and it would contradict ADR 0007’s framing of the chat UI as one MCP client among many.
- **Keep four user-selectable mount forms (FAB / Sidebar / Drawer / Palette).** The previous draft. Two of the four are invocation channels, not viewport forms; the other two are the same surface positioned differently. Conflating the axes forces a shell choice the user should not have to make.
- **Drawer as the unified desktop+mobile mount.** Rejected because a right-edge slide-over fits mobile poorly — the natural mobile gesture is bottom-anchored with drag-down dismiss. Costs the one-thumb HITL workflow for code reuse.
- **Cursor-style persistent sidebar as the only desktop mount.** Rejected because it pre-occupies main-area width even when idle, conflicting with ADR 0019's three-tab surface. A "pin sidebar open" preference can layer on the responsive mount later.
- **One chat thread per workspace, no auto-scope.** The simplest extension of the current shape. Rejected because users working across multiple workflows in the same workspace lose continuity; every conversation about workflow A mixes into the same thread as conversations about workflow B.
- **Multiple threads per scope (GitHub-issue-with-comments shape).** Considered. Rejected for now because single rolling thread per scope is simpler to ship, simpler to teach, and matches the Cursor model users already recognise. The storage schema reserves room to add a `thread_id` axis later if the multi-thread case emerges; the reserve costs nothing now.
- **Scope follows navigation strictly, no pin.** Considered for simplicity. Rejected because power users investigating workflow A while consulting workflow B’s conversation history would lose context on every click. Pin is a low-cost escape hatch (one icon, one boolean in scope state).
- **Store chat client-side (localStorage only).** Considered because it preserves the current shape. Rejected because cross-device continuity is part of the value once chat scopes get finer than workspace, and the substrate’s MCP control plane already owns conversation state authoritatively. Client-side caching can be a perf layer; authoritative state lives on the server.
- **Store chat client-side per scope, with sync-on-demand.** Considered as a hybrid. Rejected because the sync-conflict resolution adds complexity disproportionate to the value; pre-launch is the cheapest moment to fix the schema.

## Consequences

### Positive

- Chat reaches every view in the dashboard without consuming a tab slot.
- Contextual auto-scope removes the cost of restating “which workflow am I asking about” — chat already knows. Finer scope means cleaner per-object conversation history.
- One responsive mount sidesteps the "one form for everyone" debate without forcing a shell choice on the user; the five invocation channels cover the patterns different users want (push, ambient FAB, Inbox bell, scope-local Ask, keyboard).
- Server-side storage enables cross-device continuity and aligns chat state with the MCP control plane’s existing authority. Other intent frontends (Claude, Cursor) can consult the same scoped threads via the same MCP endpoints.
- Pin affordance handles the “look at A, ask about B” case without forking the conversation model.

### Negative trade-offs

- Implementation cost is non-trivial: responsive mount + four invocation channels + scope state machine + storage backend + migration of the storage call sites. Mitigation: phase 0 ships the mount plus push, FAB, bell, and scope-local Ask; phase 1 adds ⌘K. The internal `ChatContainer` is shared across phases.
- Auto-scope-on-tab-switch is subtle: scope preserves at the current level rather than resetting on every tab change. Users may not predict this. Mitigation: the chat header always shows the scope path as a breadcrumb, making the current scope unambiguous.
- Fresh-start migration discards the few existing OSS dogfood users’ chat history. Mitigation: a one-time “previous conversations” link surfaces the legacy localStorage reads for one release window; users can copy text out manually if they need to retain it.
- "More prominent entry → more specific scope" is a learned rule; the reverse would feel more natural. Mitigation: long-press on FAB jumps to Inbox, with a first-time hint on third use.
- Cross-scope history search is a new product surface (the search results UI inside chat). The first version is unlikely to be ideal. Mitigation: scoped (default) search remains the primary path; cross-scope is opt-in via an explicit search command in chat.

### Neutral

- The chat empty-state template gallery (per spec #406) is unaffected; it renders inside the responsive mount, scoped to the workspace.
- HITL cards continue to render inline in the conversation. They must fit the narrowest viewport (mobile full-width minus 16px padding) and the desktop 420px sidebar. The current card layout is already this narrow.
- The DAG preview moves from a separate right pane (the old ChatPage split) into the chat surface itself, rendered inline as a message-attached card. The visual real estate per draft is smaller; only the active draft’s DAG renders, not a permanently open preview pane.

## Dev-process counterpart

Per ADR 0002, the dev-process analog: the issue body and the PR description are the dev-process equivalents of mounted chat surfaces. The same conversation about a workflow happens both inside the dashboard chat (during build) and inside the issue or PR thread (during review). Both crystallise into the workflow’s spec.

The contextual auto-scope rule maps to: a dashboard chat scoped to `workflow:{id}` is the same conversation that lives in the spec issue’s thread for that workflow. The MCP control plane can surface either as a view onto the same thread; the storage schema’s `(user_id, scope_type='workflow', scope_id={id})` is the join key.

## Adoption checklist

Phase 0 (responsive mount + push + FAB + bell + scope-local Ask):

- [x] Land the `ChatContainer` with state machine, storage interface, conversation feed, and the Queue / Thread segmented control — one responsive shell across mobile and desktop (`components/chat/ChatContainer.tsx`, `ChatQueueView.tsx`, `ChatThreadView.tsx`)
- [x] Land the contextual auto-scope hook that observes route changes and computes scope; surface the breadcrumb in the chat header (`lib/chat/use-auto-scope.ts`, `lib/chat/scope.ts`, `components/chat/ChatBreadcrumb.tsx`)
- [x] Land the pin affordance + pinned-scope persistence (`lib/chat/pinned-scope.ts`)
- [x] Land the storage backend with the new `(user_id, scope_type, scope_id)` key shape; keep the old `localStorage` reads available behind a one-release “previous conversations” surface (`components/chat/PreviousConversationsLink.tsx`)
- [x] Land the scope-aware FAB (mobile): Active / Muted / Hidden states, long-press → Inbox, first-time hint on third use (`components/chat/ChatFab.tsx`)
- [x] Land the top-chrome bell (mobile + desktop) → Queue at `scope=workspace` (`components/chat/ChatBell.tsx`)
- [x] Land the scope-local "Ask" on step / verdict / artifact surfaces (`components/chat/ChatAskButton.tsx`)
- [x] Land push notifications with Approve / Reject for approval-kind HITL and Open-only for selection / clarify kinds; deep link scrolls Queue to the matching card (`components/chat/PushNotificationsCard.tsx`, `ChatDeepLinkHandler.tsx`)
- [x] Replace `apps/dashboard/src/pages/ChatPage.tsx` mounts in `App.tsx` with the global container; remove the `/chat` and `/workspaces/:slug/chat` page routes (the 301 redirects land in ADR 0019’s adoption checklist) — routes retired in `App.tsx` (#457); the residual `ChatPage.tsx` file is an unreferenced leftover to delete in a hygiene follow-up
- [x] Migrate the DraftStrip’s function into the Workflows tab’s `Drafted` filter chip; delete `components/chat/DraftStrip.tsx` (file removed)

Phase 1 (⌘K invocation) and closeout:

- [x] Land the ⌘K palette invocation on desktop, opening Thread at the current route's scope (`lib/chat/use-chat-ui.tsx` #473)
- [ ] Update KB page `35cd79199ca68131b4f6efac9ed8c3d6` (Onsager root) Timeline with an entry for this ADR — external (Notion); not verifiable from the repo
- [ ] Add `closed by ADR 0020` follow-up comments to specs #398 and #400 — external (GitHub); not verifiable from the repo
- [x] Flip Status to `Accepted` once phase 0 ships

## Out of scope

- **Server-side storage backend choice.** Whether the new chat state lives in Postgres, in the spine, or in a dedicated chat store is governed by the implementation spec, not this ADR. The schema shape `(user_id, scope_type, scope_id)` is fixed by this ADR; the storage engine is not.
- **Multi-thread per scope.** Reserved for a follow-up if the use case emerges; this ADR commits to single rolling thread per scope. The storage schema’s reserved-not-required `thread_id` axis keeps the door open.
- **Notification action button → MCP elicitation response binding.** This ADR fixes the *shape* of action buttons (Approve / Reject for approval-kind HITL; Open-only otherwise). The wire-level binding to the MCP elicitation response on the portal control plane (ADR 0008) — signing, replay protection, response shape when the app is closed, iOS/Android-specific affordances — is a follow-up ADR. Phase 0 may ship Open-only on all kinds if that ADR has not landed in time.
- **Chat-as-MCP-App rendering.** Whether the dashboard chat surface itself becomes an MCP App (SEP-1865), rather than a native dashboard surface that consumes MCP, is a downstream decision. KB page `361d79199ca6815d8398cc509d7fe01b` carries the open question. Not resolved here.
- **Voice or multi-modal chat input.** Out of scope.
- **Chat session sharing across users.** All conversations are user-private. Multi-user shared chat is not part of this ADR.
- **Cross-workspace chat scope.** A scope above `workspace` is not defined; chat does not span workspaces.
