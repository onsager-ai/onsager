# Onsager Dashboard

React + TypeScript + Vite UI for the Onsager factory stack. Surfaces
sessions and nodes (stiglab), governance (synodic), and factory events
(onsager-spine) in a single app.

## Run

From the repo root:

```bash
just dev             # full stack (Postgres, APIs, dashboard)
just dev-dashboard   # dashboard only
```

The dev server runs on http://localhost:5173 with HMR. It proxies API
calls to the stiglab service on `:3000` and synodic on `:3001`.

## Build & Test

```bash
pnpm --filter dashboard build
pnpm --filter dashboard test
pnpm --filter dashboard lint
```

## UI components

All form and interactive elements use [shadcn/ui](https://ui.shadcn.com/)
primitives under `src/components/ui/`. Native HTML controls
(`<input>`, `<select>`, `<button>`, `<textarea>`, `<dialog>`, etc.) are
disallowed outside the `components/ui/` directory — see the
`dashboard-ui` skill for the rule.

## Testing

- **L1** — Playwright e2e under `tests/e2e/`, run via `pnpm test:e2e`.
- **L2** — AI-driven exploratory tests, see the `web-testing` skill.
