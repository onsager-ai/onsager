---
name: web-testing
description: "L2 AI-driven web UI testing for Stiglab. Use when testing UI on PRs, triaging L1 test failures, or verifying UI behavior. Triggers include: 'test the UI', 'check the web app', 'triage test failure', 'validate this PR', 'run L2 tests', 'exploratory test'."
allowed-tools: Bash(npx agent-browser:*), Bash(agent-browser:*), Read, Write, Edit, Glob, Grep
---

# Web Testing

L2 AI-driven web UI testing for Stiglab. Validates new and changed UI
surfaces using agent-browser. Regression is L1's job (deterministic tests
in `apps/dashboard/tests/smoke/` and `tests/e2e/product/`). L2 findings get crystallized into L1.

## Inputs

L2 works from three things — no separate spec needed:
- **PR diff** (`git diff origin/main...HEAD`) — what changed, what to test
- **Existing L1 tests** (`apps/dashboard/tests/smoke/`, `tests/e2e/product/`) — what's already covered
- **agent-browser** — to explore the running UI

## Routes

| Route | Page | Key elements |
|-------|------|-------------|
| `/` | Dashboard | Overview stats, Recent Sessions table |
| `/sessions` | Sessions | All Sessions table with state badges |
| `/sessions/:id` | Session Detail | Session metadata, output log stream |
| `/nodes` | Nodes | Registered Nodes table with status badges |
| `/settings` | Settings | Profile card, Credentials (add/edit/delete forms) |

## Viewports

Every affected page MUST be tested at both viewports:

| Name | Width x Height | Description |
|------|---------------|-------------|
| `desktop` | 1280 x 720 | Default desktop viewport |
| `mobile` | 375 x 812 | Mobile (iPhone-class) viewport |

Set viewport before testing each size:
```bash
# Desktop
agent-browser set viewport 1280 720
# Mobile
agent-browser set viewport 375 812
```

## Screenshots

Screenshots are the primary evidence for L2 results. Every page at every
viewport MUST have a screenshot saved to `/tmp/l2-screenshots/`.

Naming convention: `{route_slug}-{viewport}.png`
- `/` → `dashboard-desktop.png`, `dashboard-mobile.png`
- `/sessions` → `sessions-desktop.png`, `sessions-mobile.png`
- `/sessions/:id` → `session-detail-desktop.png`, `session-detail-mobile.png`
- `/nodes` → `nodes-desktop.png`, `nodes-mobile.png`

```bash
# Create the screenshot directory
mkdir -p /tmp/l2-screenshots

# Example: capture dashboard at both viewports
agent-browser set viewport 1280 720
agent-browser batch "open http://localhost:3000" "screenshot --screenshot-dir /tmp/l2-screenshots"
mv /tmp/l2-screenshots/screenshot-*.png /tmp/l2-screenshots/dashboard-desktop.png

agent-browser set viewport 375 812
agent-browser batch "open http://localhost:3000" "screenshot --screenshot-dir /tmp/l2-screenshots"
mv /tmp/l2-screenshots/screenshot-*.png /tmp/l2-screenshots/dashboard-mobile.png
```

Include each screenshot path in the structured output `viewports[].screenshot` field.

## Execution

### 1. Read the diff

Map changed files to affected routes:
- `src/pages/*Page.tsx` → the corresponding route
- `src/components/*` → pages that use that component
- `src/hooks/*`, `src/lib/*` → all pages that import it
- `src/App.tsx`, `src/components/layout/*` → all routes

### 2. Test affected pages (both viewports)

For each affected page, at **each viewport** (desktop then mobile):
1. Set the viewport with `agent-browser set viewport`
2. Open the route and take a snapshot
3. Verify changed elements render correctly
4. Check for "undefined", "NaN", or uncaught errors
5. Check layout is not broken at the current viewport size
6. **Exercise interactive elements** (see below)
7. Take a screenshot and save it to `/tmp/l2-screenshots/`

**Interaction testing — required when the diff touches forms, buttons, or
mutations:**
- Click buttons, fill inputs, submit forms — don't just verify markup exists
- After submitting, verify the **result**: did the UI state change? Did the
  form close? Did new data appear? Did an error message show?
- Test both click-to-submit and Enter-key-to-submit for `<form>` elements
- For mutations (save/delete/update): confirm the action completes by
  checking the resulting UI state, not just that the button is present
- Screenshot **before and after** the interaction to capture evidence

**Mobile-specific checks:**
- No horizontal overflow (content fits within 375px)
- Navigation is accessible (hamburger menu, etc.)
- Tables are scrollable or have responsive layout
- Text is readable without zooming

### 3. Crystallize findings into L1

**Key output of L2.** When you validate new behavior or find a bug:

- **Validated new behavior** → write a new L1 test in
  `apps/dashboard/tests/smoke/` (component-level, vitest + Testing Library) or
  `tests/e2e/product/` (full-stack product flow) that encodes it as a
  deterministic assertion.
- **Bug found** → report it. Once fixed, write an L1 regression test.

### 4. Report

Return structured JSON matching the verdict schema. Every page entry must
include a `viewports` array with desktop and mobile entries, each containing
a `screenshot` path pointing to the saved file.

```json
{
  "verdict": "PASS",
  "summary": "All 3 affected pages pass at desktop and mobile viewports",
  "pages_tested": [
    {
      "route": "/",
      "status": "PASS",
      "notes": "Dashboard renders correctly at both viewports",
      "viewports": [
        { "name": "desktop", "status": "PASS", "screenshot": "/tmp/l2-screenshots/dashboard-desktop.png" },
        { "name": "mobile", "status": "PASS", "screenshot": "/tmp/l2-screenshots/dashboard-mobile.png" }
      ]
    }
  ],
  "issues": [],
  "crystallized": ["tests/smoke/session-filter.test.tsx (3 tests)"]
}
```

## L1 triage mode

When triggered by L1 failure:

1. Read the failing test to understand the expected assertion
2. Open the relevant page with agent-browser
3. Snapshot actual state vs expected
4. Diagnose: real regression, environment issue, or flaky test
5. Report with evidence + suggested fix
