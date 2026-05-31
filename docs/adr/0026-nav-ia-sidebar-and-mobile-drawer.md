# ADR 0026 — Nav IA: desktop sidebar + mobile drawer

- **Status**: Accepted
- **Date**: 2026-05-31
- **Identity impact**: no
- **Adoption**: enforced
- **Tracking issues**: #519 (implementation spec). Carries the nav half of #517 (L2-testing findings). Lands folded into PR #518 alongside the `/mcp` dev-proxy fix.
- **Supersedes**: ADR 0019 (dashboard IA: pipeline-centric three-tab surface) — the top-bar-with-tabs chrome it adopted.
- **Superseded by**: none

## Context

ADR 0019 adopted a pipeline-centric top-level IA: the sidebar was removed
and the section nouns became tabs in the top chrome, with the workspace
switcher and user menu beside them. The section set has since grown from
three to four+ (`Plans` was added as a sanctioned fifth noun, ADR 0023 /
0025).

That chrome can only express the section nouns. Every other destination
falls outside it:

- **Global Settings** (`/settings`), the **Workspaces list**
  (`/workspaces`), and **per-workspace Settings**
  (`/workspaces/:slug/settings`) match none of the tabs, so the nav shows
  **no active item** on those pages — disorienting "where am I?" state.
- On the global pages the four tabs still render but point at
  `/workspaces` (inert), and the switcher shows a "Select workspace"
  placeholder — clickable-but-dead chrome.
- On mobile the top row carries title + a horizontally-scrolling tab
  strip competing for ~375px, and Settings/Workspaces have no home there
  at all.

This surfaced during L2 UI testing (#517). The root shape is structural:
a flat tab bar cannot host destinations that aren't sections.

## Decision

Adopt a **persistent left sidebar on desktop** and a **left off-canvas
drawer on mobile**, rendered from **one shared `SidebarBody`**.

`SidebarBody` (`apps/dashboard/src/components/layout/AppLayout.tsx`):
logo → workspace switcher → section links (vertical, active-highlighted)
→ a first-class **Settings** entry → the account / chat / ⌘K footer
cluster. Every destination — sections *and* Settings — gets an
unambiguous active state. The dead-active-state and inert-tab problems
disappear because nav is no longer constrained to the section set.

- **Desktop:** `SidebarBody` inside a `w-60` `<aside>` rail.
- **Mobile:** `SidebarBody` inside a left `Sheet` drawer, opened by a
  hamburger in `MobileHeader`. Drill-down pages (those that set
  `usePageHeader({ backTo })`) keep the back arrow instead of the
  hamburger. The drawer closes on nav-link selection (`onNavigate`) and
  on backdrop tap.

Nav items are plain `<Link>`s styled with `buttonVariants`, not
`<Button render={<Link/>}>`. This keeps the correct `role="link"`
semantics for navigating elements and avoids Base UI's `nativeButton`
console warning — the recurring nag #517 flagged. (Setting
`nativeButton={false}` on a `Button` is the wrong fix: it stamps
`role="button"` onto the anchor.)

The chat surface (ADR 0020 / 0025) is unchanged: the desktop labeled
entry, the bell, and the mobile FAB all move into the new chrome as-is.

## Rejected alternatives

- **Mobile bottom tab bar.** Prototyped first (thumb-reachable, always
  visible). Rejected in review in favour of the drawer pattern: the
  drawer hosts the *full* sidebar body (sections + Settings + workspace
  switcher + account) in one place, matching the desktop rail exactly,
  whereas a bottom bar can only carry the section nouns and re-splits
  Settings/account elsewhere — reintroducing the asymmetry this ADR
  exists to remove.
- **Keep the top tabs, add a Settings tab.** Settings is not a section
  noun (CLAUDE.md vocabulary); promoting it to a top-level tab muddies
  the four-noun surface. The rail/drawer gives it a footer home without
  making it a peer of the sections.

## Adoption checklist

- [x] `SidebarBody` / `DesktopSidebar` / `MobileNavDrawer` implemented;
      top-tab rows removed.
- [x] `top-chrome.test.tsx` updated to the hamburger-drawer behaviour
      (mutation-checked).
- [x] ADR 0019 marked superseded by this ADR.
- [x] `dashboard-ui` skill (in-repo, `.claude/skills/`) reconciled: the
      mobile-bar diagram and account-footer component references now
      match the shipped `SidebarBody` / `DesktopSidebar` /
      `MobileNavDrawer`. (The skill already prescribed the sidebar +
      `☰` drawer; ADR 0019's top-tabs had diverged from it.)
