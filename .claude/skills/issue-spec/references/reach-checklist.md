# Reach checklist

When a spec introduces a new user-facing primitive — a resource a user
must create (workspace, project, credential) or a new capability with a
UI entry point (governance action, artifact type, session kind) — the
spec's Plan section must include the discovery surface, not just the
CRUD.

Without this, the primitive ships as dead code: functionally live but
invisible. The cheap option (ship the API + a hidden Settings card,
defer the surface to a follow-up spec) is almost always the wrong call —
see the workspace onboarding case in #70 for the canonical example.

## Items to scope into the spec Plan

Copy the applicable items into the spec's Plan section. Not every
primitive needs every item — a governance action probably doesn't need
a sidebar entry — but a *user-created resource* typically needs all of
them.

### Discovery

- [ ] **Primary navigation entry** in `AppSidebar.tsx` under the right
  group. Create a new group if the category doesn't exist — don't bury
  under "System" or "Settings."
- [ ] **Dedicated page** (`/<primitive>s`), not just a card inside
  another page. Lists, create flow, and empty state live here.
- [ ] **First-run redirect** for authenticated users with zero
  instances. See `OnboardingGate` in `App.tsx` as the canonical pattern.
  Session-scoped dismissal via `sessionStorage` so subsequent navigation
  doesn't loop.

### Empty states

- [ ] **Stepped onboarding hero** on the primitive's own page when the
  user has zero instances. Two or three numbered steps max, with an
  active CTA on step one.
- [ ] **Empty-state CTA** on any *other* page that expects instances to
  exist (e.g. Factory Overview shows a "Set up workspace" banner when
  the user has no workspaces). Always a button linking to the primitive's
  page, not a paragraph of instructions.

### Create affordances

- [ ] **`QuickCreateMenu` entry** if the resource is create-intensive.
- [ ] **Create affordance where the user already looks** — if the primitive
  is typically created in the flow of another action (e.g. linking a
  GitHub install from the workspace page), wire that path, not just the
  standalone modal.

### Hygiene

- [ ] **Auth gating.** Every query and entry point gated on
  `authEnabled && user`. Anonymous/L1 contexts must not 401 or surface a
  dead CTA. If the primitive's visibility depends on auth, hide the
  sidebar entry too — a visible link to a broken page is worse than no
  link.
- [ ] **Client validation mirrors server rules.** If the backend rejects
  a pattern (slug regex, length, reserved words), the client normalizes
  or shows an inline error before submit. Helper text that can't prevent
  a 400 is a lie.
- [ ] **Secondary link-out** from related surfaces (e.g. Settings page
  links to `/workspaces`) for users who search in the old place. One
  link out, not an embedded copy of the UI.

## What's explicitly deferrable

These are fine to scope *out* with a Non-goals section, as long as the
items above are in:

- Switcher / active-instance context indicator (only when ≥1 page
  consumes the context globally).
- Role editors, invites, billing, quotas.
- Bulk operations, templates, import/export.
- Cascading delete / archive flows.

If the spec's Non-goals covers these explicitly, a follow-up spec is
legible. What's *not* fine to defer: the user's ability to find the
primitive at all.

## How to verify

Before marking the spec `planned`:

1. Walk through the flow as a brand-new user with empty database state.
2. If at any point there's a page with no content and no actionable
   CTA, you're missing a Reach item.
3. Repeat with `authEnabled=false` — anonymous / L1 mode must not
   surface dead entry points.

The `web-testing` skill exercises both states in its exploratory pass
and will catch regressions.
