# Onsager

Workflow automation that runs AI agent sessions and turns their output
into trusted artifacts. One process, one database, one dashboard.

Create a **workflow** (a trigger plus agent stages), fire it manually or
from a GitHub issue label, and Onsager runs a Claude agent session whose
deliverables land as **artifacts** — content-addressed, provenance-tagged,
attributed to their **run**, with the full audit timeline alongside.

## Quick start

Prerequisites: Docker, Rust toolchain (rustup), pnpm.

```bash
cp .env.example .env     # set ONSAGER_CREDENTIAL_KEY (openssl rand -hex 32)
just setup               # git hooks + pnpm install
just dev                 # Postgres + the onsager process + Vite
```

Open the dashboard (the `just dev` output prints the port), dev-login,
add `CLAUDE_CODE_OAUTH_TOKEN` under **Credentials**, create a workflow,
and hit **Run**.

## How it works

- One binary serves the dashboard API, the GitHub App webhook, and the
  agent-facing MCP endpoint, and hosts the scheduler in-process — a
  fire is a function call.
- Agent sessions are `claude` CLI subprocesses that deliver via the MCP
  `emit_artifact` tool; output lands as artifact rows with provenance.
- Postgres holds all durable truth; every run appends an audit trail to
  the `events` table (the record, not the protocol — ADR 0029).

## Documentation

- [`CLAUDE.md`](CLAUDE.md) — architecture, identity commitments, schema,
  conventions (also the agent-facing project instructions).
- [`docs/adr/`](docs/adr/) — architecture decision records
  ([index](docs/adr/README.md)); ADR 0029 explains the 0.5 reset that
  produced this codebase.

## License

AGPL-3.0.
