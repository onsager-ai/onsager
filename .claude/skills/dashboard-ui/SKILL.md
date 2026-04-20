---
name: dashboard-ui
description: Enforce shadcn/ui component usage in apps/dashboard AND the "avoid manual input" UX principle — linkable fields (repo owner/name, installation IDs, project slugs, URLs) must be solved with OAuth pickers or deep-links-out, never typed inputs. Native HTML form and interactive elements (input, select, button, textarea, checkbox, radio, dialog, etc.) are forbidden — use the shadcn/ui primitives under @/components/ui instead. Trigger when editing or creating .tsx files under apps/dashboard/, when adding forms/buttons/inputs/selects/modals to the web app, when wiring up GitHub/Railway/OAuth integrations, or when the user mentions "shadcn", "UI component", "form control", "dashboard component", "manual input", "paste URL", "linkable field", "borrow UX from Railway/Vercel/Claude".
---

# dashboard-ui

The Onsager dashboard (`apps/dashboard`) standardises on **shadcn/ui** for all
interactive and form UI. Native HTML elements for these controls are not
allowed — they bypass the design system, theming (next-themes + CSS variables),
and accessibility behavior baked into the shadcn primitives.

## The rule

In any `.tsx` under `apps/dashboard/src/` — **except** for the shadcn
primitives themselves in `apps/dashboard/src/components/ui/**`, which
legitimately wrap native elements:

- **Do not** write `<input>`, `<select>`, `<textarea>`, `<button>`,
  `<option>`, native `<dialog>`, native checkbox/radio `<input type="...">`,
  or an unstyled `<a>` that behaves like a button.
- **Do** import the shadcn equivalent from `@/components/ui/<name>`.
- Plain structural/text tags (`<div>`, `<span>`, `<p>`, `<h1>`–`<h6>`,
  `<ul>`, `<li>`, `<form>`, `<label>`, `<a>` for real navigation, etc.) are
  fine — the rule is specifically about interactive / form controls.

## Mapping

| Native                                  | Use instead                                     |
| --------------------------------------- | ----------------------------------------------- |
| `<button>`                              | `Button` from `@/components/ui/button`          |
| `<input type="text\|email\|...">`       | `Input` from `@/components/ui/input`            |
| `<input type="checkbox">`               | `Checkbox` from `@/components/ui/checkbox`      |
| `<input type="radio">`                  | `RadioGroup` from `@/components/ui/radio-group` |
| `<textarea>`                            | `Textarea` from `@/components/ui/textarea`      |
| `<select>` / `<option>`                 | `Select` from `@/components/ui/select`          |
| `<dialog>` / custom modal               | `Dialog` from `@/components/ui/dialog`          |
| Drawer / off-canvas                     | `Sheet` from `@/components/ui/sheet`            |
| Tooltip                                 | `Tooltip` from `@/components/ui/tooltip`        |
| Dropdown menu                           | `DropdownMenu` from `@/components/ui/dropdown-menu` |
| Tabs                                    | `Tabs` from `@/components/ui/tabs`              |
| Table                                   | `Table` from `@/components/ui/table`            |

## Installed components

These are already present under `apps/dashboard/src/components/ui/`:

`badge`, `button`, `card`, `command`, `dialog`, `dropdown-menu`, `input`,
`input-group`, `popover`, `scroll-area`, `select`, `separator`, `sheet`,
`sidebar`, `skeleton`, `table`, `tabs`, `textarea`, `tooltip`.

If you need a component that isn't in the list (e.g. `dialog`, `checkbox`,
`radio-group`, `form`, `switch`), add it with:

```bash
cd apps/dashboard
pnpm dlx shadcn@latest add <name>
```

The CLI writes the file to `src/components/ui/<name>.tsx` using
`components.json` (style `base-nova`, neutral base color, `@/components/ui`
alias). Commit the generated file alongside the change that uses it.

## Checking existing code

Before claiming a dashboard change is done, grep for violations in files you
touched:

```bash
rg -n '<(button|input|select|textarea|option|dialog)[ />]' apps/dashboard/src \
  --glob '!apps/dashboard/src/components/ui/**'
```

Any hit in your diff should be replaced with the shadcn primitive.

## Why

- Consistent theming via CSS variables + `next-themes` dark mode.
- Keyboard / focus / ARIA handling lives in the primitive, not in each call site.
- Styling lives in one place (the `ui/` component) — ad-hoc native elements
  diverge quickly and are expensive to re-skin later.

## UX principle: avoid manual input, streamline everything

When designing any flow that touches external systems (GitHub, Railway,
deploy providers, cloud accounts), default to the patterns established
platforms like Railway, Vercel, and Claude use — not "show a form with a
bunch of fields to type into." On mobile especially, typing owner/name,
IDs, URLs, or secrets is a dead-end UX.

**Order of preference, highest to lowest:**

1. **OAuth / App install** — one button, platform owns identity. Deep-link
   out and come back via redirect; no IDs or secrets touch the user.
2. **Searchable picker (combobox) from already-authorised data** — a
   `Popover` + `Command` (cmdk) combobox populated from the linked
   install, with typeahead filtering. Never a plain `Select` once a list
   can grow past a handful — on mobile, scrolling 100+ flat `SelectItem`s
   is a dead-end. The shadcn primitives are in
   `@/components/ui/{popover,command}` (installed via `pnpm dlx shadcn@latest add command popover`).
3. **Deep-link back to the source of truth** — when the picker doesn't
   contain what the user wants, link out ("Configure repository access on
   GitHub →") instead of showing a manual form. Use the platform's own
   settings page — it owns the state.
4. **Pasted URL** — acceptable only when the platform has no App/OAuth
   model for the resource (rare for GitHub; common for one-off public
   links). Still prefer a picker.
5. **Typed identifiers / split fields (owner + name, id + login, etc.)** —
   **do not use.** If you find yourself writing two `<Input>` fields for
   something a user has in a URL bar, you're building the wrong UI.

**Freestyle input is never acceptable when the domain is known.** If the
set of valid values is finite and we can enumerate it (repos on an
install, nodes online, branches in a repo, members of a workspace), the
input must be a **search + selection** combobox, not a free-text field.
The minimum bar is: type to filter, click to select, backed by an actual
data source — never a bare `<Input>` with validation hoping the user
typed the right thing.

**Concrete patterns to follow:**

- When a GitHub App installation exists but the desired repo isn't in the
  accessible-repos list, surface a **"Configure repository access on
  GitHub →"** deep link to
  `https://github.com/organizations/<login>/settings/installations/<install_id>`
  (org) or `https://github.com/settings/installations/<install_id>` (user).
  Vercel, Railway, and Render all use this pattern.
- When an App credential isn't configured server-side, show an
  informational message ("Ask an administrator to set up the App") rather
  than a manual-entry fallback form. A half-working manual path is worse
  than a clearly-blocked one.
- Infer derived values (default branch, account type, etc.) from the
  picker's payload — never ask the user to repeat what the API already
  knows.

**Concrete patterns to avoid:**

- A form that asks for "Installation ID (numeric) + Account login +
  Account type + Webhook secret" — that's engineering plumbing leaking
  into UI. OAuth is the only acceptable path for App install linking.
- A "paste a repo URL" or "enter owner/name manually" escape hatch next
  to an OAuth-backed picker. The deep-link-out pattern is strictly better.
- Separate inputs for values that live together in a single URL or
  identifier (e.g. `owner` + `name` for a GitHub repo). Either parse from
  a paste, or don't ask at all.

### Anti-pattern: input box for linkable fields

Any value that the user can obtain by **clicking a link** on another page
is a "linkable field." These are almost always wrong to ask for as a typed
input — the user has to context-switch, copy, paste, and verify. Every
step is a mobile-hostile tax and an opportunity for a typo.

**If a field is linkable, do not use an `<Input>` for it.** Instead:

- **Pick it from a list** the linked system already hands us (repos,
  branches, environments, service names, deploy targets).
- **Deep-link out** to where the user can grant or configure it, then
  re-fetch when they return.
- **Infer it** from another selection (default branch from repo, account
  type from install payload, organisation from user session).

Linkable fields include, but aren't limited to:

- GitHub repo identifiers: owner, name, full URL, SSH URL, default branch,
  PR number, issue number, installation id.
- Cloud resource identifiers: project id, service id, region, environment
  slug, secret reference.
- Third-party account identifiers: workspace slug, team id, seat id,
  org login.

**Rule of thumb:** if you're about to label an input "Installation ID",
"Repo owner", "Project slug", or anything that is already a hyperlink on
the source platform's dashboard, stop and redesign. Use a picker, a
deep-link, or inference. Typing one of these is never the right UX — not
on desktop, and definitely not on mobile.

When in doubt, open Railway/Vercel, walk through the analogous flow, and
copy the pattern.

## New primitive = new surface

When the dashboard grows a new user-facing resource (workspace, project,
credential, node type — anything a user can create), shipping just the
CRUD UI isn't enough. A primitive without entry points is functionally
invisible: users land on empty pages with no hint that the resource
exists, and the feature ships as dead code.

**This is the product-side complement to the `issue-spec` "Reach ships
with the primitive" principle** — the spec scopes these in, the UI
implementation checks them off. If a spec lands on your desk without
them, push back at the spec stage. Don't quietly defer.

**Before a PR that introduces a new primitive is ready:**

- [ ] **Sidebar entry** in `AppSidebar.tsx` under the right group. Create
  a new group (e.g. "Organization") if the category doesn't exist —
  don't bury under "System" or "Settings."
- [ ] **Dedicated page** at `/<primitive>s`, not just a card inside
  Settings. Lists, create flow, and the empty state all live here.
- [ ] **First-run redirect** for authenticated users with zero
  instances. Pattern: `OnboardingGate` in `App.tsx`. Session-scoped
  dismissal via `sessionStorage` so navigating away doesn't loop.
- [ ] **Stepped onboarding hero** on the dedicated page when empty.
  Two or three numbered steps, active CTA on step one.
- [ ] **Empty-state CTA** on *other* pages that expect instances to
  exist (e.g. a workspace-setup banner on Factory Overview). Always a
  button linking to the primitive's page — never a paragraph of
  instructions.
- [ ] **`QuickCreateMenu` entry** if the resource is create-intensive.
- [ ] **Auth gating on every query and entry point**
  (`enabled: authEnabled && !!user`). Anonymous / L1-smoke contexts
  must not 401 or render a dead CTA. If the primitive requires auth,
  hide the sidebar entry too — a visible link to a broken page is
  worse than no link.
- [ ] **Client validation mirrors server rules** (slug regex, length,
  etc.). Normalize on input or show an inline error before submit —
  helper text that can't prevent a 400 is a lie.

**The cheap option is usually wrong.** It's tempting to ship just the
list + create card inside Settings and plan the surface as a follow-up.
In practice the follow-up PR is bigger than building it up front, the
primitive is invisible in the meantime, and reviewers/users start
reporting "there's no way to do X" despite the code being live.
See #70 / `/workspaces` as the canonical example of what this shape
looks like when done right.
