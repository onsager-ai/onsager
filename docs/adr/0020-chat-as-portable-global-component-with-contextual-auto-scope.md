# ADR 0020 — Chat as portable global component with contextual auto-scope

- **Status**: Proposed
- **Date**: 2026-05-22
- **Identity impact**: no
- **Tracking issues**: #451 (implementation spec). Sibling to
  ADR 0019, which removes the top-level Chat tab. This ADR governs
  what chat *becomes* once it is no longer a tab. Builds on ADR 0007
  (tools and skills as the public contract) and ADR 0008 (portal
  owns the agent control plane).
- **Supersedes**: none. Predecessor specs (#398 universal-chat-entry,
  #400 chat empty-state chips, #401 DraftStrip + chat surface) are
  the prior frame in prose; the repo convention reserves the
  `Supersedes` field for ADR-to-ADR replacement, so no entry here.
- **Superseded by**: none

## Context

The dashboard’s chat surface is a page today — `ChatPage.tsx` (1005
lines), reachable via two routes (`/chat` and
`/workspaces/:slug/chat`). It hosts a conversation feed with HITL
cards, a draft chip strip, a right-pane DAG preview, an empty-state
template gallery, and a stack of banners (OSS notice, webhook health
warning). The right-pane DAG preview is rendered as a 60% viewport
strip even when no draft is active.

This commits the chat to one of the four top-level dashboard surfaces
that spec #289’s two-surface IA recognized. With ADR 0019 collapsing
the IA to three tabs (Workflows / Runs / Activity) and removing Chat
from top-level, the question becomes: how does a user reach chat, and
in what form?

KB position (Notion `35cd79199ca68131b4f6efac9ed8c3d6`, Onsager root
note) frames the substrate as UI-portable via the MCP control plane:

> Onsager substrate UI-portable via MCP control plane (intent /
> spec_plan / compile / run / status / artifacts / verdicts); Claude
> / Cursor / Onsager UI 互换 intent frontends

ADR 0007 names the same point at the protocol layer: tools and skills
are the public contract; the dashboard chat UI is one of N consumers
of that contract, not a privileged one. ADR 0008 then carves portal
as the owner of the public surface those consumers reach. Together
they imply that chat inside the dashboard:

- Is one MCP client among Claude, Cursor, and any other intent
  frontend
- Should not occupy a privileged top-level position relative to other
  user activities
- Should be reachable from anywhere a user might form an intent
- Should be aware of what the user is currently looking at, without
  forcing them to restate it

A second concern is form. Power-user preferences genuinely diverge.
Some want chat permanently visible (Cursor-style right sidebar). Some
want it on-demand and out of the way when idle (Linear-style slide-
over drawer). Some want minimum chrome and keyboard-only entry
(Raycast-style palette mode). Some want a low-commitment touchpoint
that never blocks the main view (Intercom-style floating button).
Forcing one form across all users is arbitrary, and the underlying
conversation state is mount-agnostic — the same `ChatContainer` can
render inside any of them.

The third concern is scope. The current
`chatStorageKey(user.id, workspace.id)` at
`apps/dashboard/src/pages/ChatPage.tsx:293-297` ties one thread per
workspace. Users investigating multiple workflows in the same
workspace lose continuity: every conversation about workflow A is
mixed into the same thread as conversations about workflow B. A
finer scoping is needed.

## Decision

Chat becomes a portable global component, mountable in four user-
selectable forms with one shared state and storage layer.

### Mount forms

|Form       |Position                                           |Default trigger         |Typical user               |
|-----------|---------------------------------------------------|------------------------|---------------------------|
|**FAB**    |Floating bottom-right button + expanding panel     |Click FAB               |Low-commitment touchpoint  |
|**Sidebar**|Persistent right column (desktop) / drawer (mobile)|Always-on               |Cursor-style power user    |
|**Drawer** |Slide-over panel from the right edge               |Top-chrome button + `⌘J`|Default; “open when needed”|
|**Palette**|Embedded inside ⌘K as a chat mode                  |`⌘K` then `tab`         |Keyboard-only              |

**Drawer is the default** for new accounts. The choice is conservative:
it does not pre-occupy main-area width and adapts cleanly from
desktop to mobile. Users change it via account-level settings or a
dock-mode toggle in the top chrome that cycles between forms at
runtime. The toggle does not lose conversation state — switching the
mount only changes the shell.

The four mounts share a single `ChatContainer` component (state +
storage + conversation feed + HITL card layout + DAG preview). Mount
wrappers are thin adapters that handle position, sizing, and dismissal
behaviour.

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

Each scope holds **one rolling thread**. Returning to a previously
visited scope resumes the same conversation; no implicit thread
creation. A pin control in the chat header locks the scope against
navigation changes, supporting cross-context inquiry — “looking at
workflow A while asking a question about workflow B” — without
forking the conversation model.

Cross-scope history is fully indexed on the backend. The scope filter
is a default view, not a wall: when the user asks “what did we
decide about release notes last week?” inside a workflow A scope, the
assistant searches across all the user’s scopes in the current
workspace and cites the source scope alongside the answer.

The chat header renders the scope path as a breadcrumb:
`acme-corp / Auto-merge on green / Run #42`. Pin state shows as a
filled-icon affordance next to the breadcrumb.

### Storage schema change

```
old: chatStorageKey(user_id, workspace_id) → localStorage entry
new: chatStorageKey(user_id, scope_type, scope_id) → backend (server-side)
```

The old single-thread-per-workspace key collapses to
`scope_type='workspace'`; new finer scopes (`workflow`, `run`,
`verdict`) get their own keys. The storage backend moves from
client-side `localStorage` to server-side persistence; the engine
choice (Postgres, spine, dedicated chat store) is delegated to the
implementation spec.

**Migration strategy: fresh start.** Onsager is pre-launch; existing
chat-storage entries belong to OSS dogfood users only. Their
`localStorage` entries are read once at first login after this ADR
ships and surfaced as a one-time “previous conversations” link
inside the workspace-scoped thread. Subsequent writes go to the
backend under the new schema. No automated data migration ships.

### Empty-state behaviour

After ADR 0019 + this ADR land, signed-in users with zero workflows
land on the Workflows tab and see an empty state. The empty state
displays a “Start in chat →” card that opens the chat surface in
whichever mount the user has chosen, scoped to the workspace. The
chat empty state itself shows the template gallery from
`apps/dashboard/src/lib/templates/v0.json` (per spec #406, content
unchanged). The DraftStrip from `apps/dashboard/src/components/chat/ DraftStrip.tsx` is removed; its function (switching between in-flight
drafts) is absorbed into the Workflows tab’s state-filter chips when
filtered to `Drafted`.

## Rejected alternatives

- **Keep chat as a page (one of the three top-level tabs).**
  Considered as a “Chat / Workflows / Runs” tab trio. Rejected
  because it would re-create the spec #289 mode-split that pipeline-
  centric IA explicitly retires, and it would contradict ADR 0007’s
  framing of the chat UI as one MCP client among many.
- **Single mount form (Sidebar only, or Drawer only).** Considered
  for each of the four. Rejected because user preferences genuinely
  diverge across the four established patterns (Cursor / Linear /
  Raycast / Intercom), and the conversation state is mount-agnostic.
  Supporting all four costs four thin wrappers around one container,
  not four implementations.
- **One chat thread per workspace, no auto-scope.** The simplest
  extension of the current shape. Rejected because users working
  across multiple workflows in the same workspace lose continuity;
  every conversation about workflow A mixes into the same thread as
  conversations about workflow B.
- **Multiple threads per scope (GitHub-issue-with-comments shape).**
  Considered. Rejected for now because single rolling thread per
  scope is simpler to ship, simpler to teach, and matches the Cursor
  model users already recognise. The storage schema reserves room to
  add a `thread_id` axis later if the multi-thread case emerges; the
  reserve costs nothing now.
- **Scope follows navigation strictly, no pin.** Considered for
  simplicity. Rejected because power users investigating workflow A
  while consulting workflow B’s conversation history would lose
  context on every click. Pin is a low-cost escape hatch (one
  icon, one boolean in scope state).
- **Store chat client-side (localStorage only).** Considered because
  it preserves the current shape. Rejected because cross-device
  continuity is part of the value once chat scopes get finer than
  workspace, and the substrate’s MCP control plane already owns
  conversation state authoritatively. Client-side caching can be a
  perf layer; authoritative state lives on the server.
- **Store chat client-side per scope, with sync-on-demand.**
  Considered as a hybrid. Rejected because the sync-conflict
  resolution adds complexity disproportionate to the value;
  pre-launch is the cheapest moment to fix the schema.

## Consequences

### Positive

- Chat reaches every view in the dashboard without consuming a tab
  slot.
- Contextual auto-scope removes the cost of restating “which workflow
  am I asking about” — chat already knows. Finer scope means cleaner
  per-object conversation history.
- Mount-form preference becomes a personal choice. The “one form for
  everyone” UX debate goes away; users opt into their style.
- Server-side storage enables cross-device continuity and aligns
  chat state with the MCP control plane’s existing authority. Other
  intent frontends (Claude, Cursor) can consult the same scoped
  threads via the same MCP endpoints.
- Pin affordance handles the “look at A, ask about B” case without
  forking the conversation model.

### Negative trade-offs

- Implementation cost is non-trivial: four mount wrappers + scope
  state machine + storage backend + migration of the storage call
  sites. Mitigation: phase 0 ships Drawer + Sidebar only; phase 1
  adds FAB; phase 2 adds Palette. The internal `ChatContainer`
  component is shared across phases.
- Auto-scope-on-tab-switch is subtle: scope preserves at the current
  level rather than resetting on every tab change. Users may not
  predict this. Mitigation: the chat header always shows the scope
  path as a breadcrumb, making the current scope unambiguous.
- Fresh-start migration discards the few existing OSS dogfood users’
  chat history. Mitigation: a one-time “previous conversations” link
  surfaces the legacy localStorage reads for one release window;
  users can copy text out manually if they need to retain it.
- The default mount (Drawer) is a judgment call, not a measured
  decision. Some users will dislike it. Mitigation: the dock-mode
  toggle is in the top chrome, one click away, and settings carry
  the preference across sessions.
- Cross-scope history search is a new product surface (the search
  results UI inside chat). The first version is unlikely to be ideal.
  Mitigation: scoped (default) search remains the primary path;
  cross-scope is opt-in via an explicit search command in chat.

### Neutral

- The chat empty-state template gallery (per spec #406) is unaffected;
  it renders inside whichever mount the user picks.
- HITL cards continue to render inline in the conversation. They
  must fit in the narrowest mount (FAB panel, ~360px wide). The
  current card layout is already this narrow.
- The DAG preview moves from a separate right pane (the old ChatPage
  split) into the chat surface itself, rendered inline as a
  message-attached card. The visual real estate per draft is
  smaller; only the active draft’s DAG renders, not a permanently
  open preview pane.

## Dev-process counterpart

Per ADR 0002, the dev-process analog: the issue body and the PR
description are the dev-process equivalents of mounted chat surfaces.
The same conversation about a workflow happens both inside the
dashboard chat (during build) and inside the issue or PR thread
(during review). Both crystallise into the workflow’s spec.

The contextual auto-scope rule maps to: a dashboard chat scoped to
`workflow:{id}` is the same conversation that lives in the spec
issue’s thread for that workflow. The MCP control plane can surface
either as a view onto the same thread; the storage schema’s
`(user_id, scope_type='workflow', scope_id={id})` is the join key.

## Adoption checklist

- [ ] Land the `ChatContainer` component with state machine, storage
  interface, and conversation feed — mount-form-agnostic
- [ ] Land Drawer mount (default) and Sidebar mount as phase 0
- [ ] Land the contextual auto-scope hook that observes route changes
  and computes scope; surface the breadcrumb in the chat header
- [ ] Land the pin affordance + pinned-scope persistence
- [ ] Land the storage backend with the new
  `(user_id, scope_type, scope_id)` key shape; keep the old
  `localStorage` reads available behind a one-release “previous
  conversations” surface
- [ ] Replace `apps/dashboard/src/pages/ChatPage.tsx` mounts in
  `App.tsx` with the global container; remove the `/chat` and
  `/workspaces/:slug/chat` page routes (the 301 redirects land
  in ADR 0019’s adoption checklist)
- [ ] Add the dock-mode toggle to top chrome and the form selector
  to account settings
- [ ] Migrate the DraftStrip’s function into the Workflows tab’s
  `Drafted` filter chip; delete `components/chat/DraftStrip.tsx`
- [ ] Land FAB mount (phase 1)
- [ ] Land Palette mount (phase 2)
- [ ] Update KB page `35cd79199ca68131b4f6efac9ed8c3d6` (Onsager
  root) Timeline with an entry for this ADR
- [ ] Add `closed by ADR 0020` follow-up comments to specs #398 and
  #400
- [ ] Flip Status to `Accepted` once phase 0 (Drawer + Sidebar +
  storage + auto-scope) ships

## Out of scope

- **Server-side storage backend choice.** Whether the new chat state
  lives in Postgres, in the spine, or in a dedicated chat store is
  governed by the implementation spec, not this ADR. The schema
  shape `(user_id, scope_type, scope_id)` is fixed by this ADR; the
  storage engine is not.
- **Multi-thread per scope.** Reserved for a follow-up if the use
  case emerges; this ADR commits to single rolling thread per scope.
  The storage schema’s reserved-not-required `thread_id` axis keeps
  the door open.
- **Chat-as-MCP-App rendering.** Whether the dashboard chat surface
  itself becomes an MCP App (SEP-1865), rather than a native
  dashboard surface that consumes MCP, is a downstream decision.
  KB page `361d79199ca6815d8398cc509d7fe01b` carries the open
  question. Not resolved here.
- **Voice or multi-modal chat input.** Out of scope.
- **Chat session sharing across users.** All conversations are
  user-private. Multi-user shared chat is not part of this ADR.
- **Cross-workspace chat scope.** A scope above `workspace` is not
  defined; chat does not span workspaces.
