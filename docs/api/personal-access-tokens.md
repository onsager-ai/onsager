# Personal Access Tokens

Personal Access Tokens (PATs) are user-owned bearer credentials that
authenticate non-browser callers — CLIs, CI jobs, agents, notebooks —
against the same `/api/*` surface the dashboard uses. Issued in
[#143](https://github.com/onsager-ai/onsager/issues/143).

## Token format

```
ons_pat_<32 url-safe random bytes>
```

The `ons_pat_` namespace prefix matches GitHub's `ghp_` style so secret
scanners can match on it. The full token is shown to the user **exactly
once**, on creation. The server stores only the SHA-256 hash plus the first
12 characters (the `token_prefix`) for display + indexed lookup; there is
no decrypt path, even for the owner.

## Header shape

```
Authorization: Bearer ons_pat_<...>
```

A request that carries both a valid PAT and a valid `stiglab_session`
cookie is authenticated as the PAT user — the bearer wins, so a CLI smoke
test from a developer's terminal can't silently fall through to the
browser session that happens to be in scope.

`/api/auth/me` includes a `via` field so callers can confirm which path
authenticated the request:

```json
{ "user": { ... }, "auth_enabled": true, "via": "pat" }
```

## Lifecycle

| Operation | Endpoint | Notes |
| --- | --- | --- |
| List | `GET /api/pats` | Prefix-only metadata. The full token is **not** returned. |
| Create | `POST /api/pats` | Returns the full token in the response body, **once**. |
| Revoke | `DELETE /api/pats/{id}` | Soft-delete (sets `revoked_at`). Audit row is kept. |

Create body:

```json
{
  "name": "ci",
  "tenant_id": null,
  "expires_at": "2026-07-25T00:00:00Z"
}
```

* `name` is required, must be unique per user.
* `tenant_id` is optional. When set, the PAT is **pinned** to that
  workspace — calls touching another tenant return 403
  `pat_tenant_scope_mismatch`. The dashboard form defaults to "All
  workspaces".
* `expires_at` is required from the API. The dashboard offers GitHub-style
  choices (7 / 30 / 60 / 90 days / custom date). `null` is reserved for a
  future "never expires" affordance and not exposed in v1.

A revoked or expired PAT returns:

```
HTTP/1.1 401 Unauthorized
WWW-Authenticate: Bearer error="invalid_token"
```

## Destructive-operation guardrail

PATs authenticate the same user as a session, but the API treats them as a
**lower-privilege principal** for destructive credential operations:

* `DELETE /api/credentials/{name}` returns 403 `pat_destructive_blocked`.
* `PUT /api/credentials/{name}` is allowed only when the credential does
  **not** already exist (create-only); overwriting an existing credential
  returns the same 403.
* All other endpoints (sessions, tasks, governance, tenants, projects,
  list/read credentials) behave as for a session.

The intent is that rotating a stored secret stays a deliberate
human-in-the-browser action, even when an automated PAT-bearing pipeline
has full access otherwise.

## Curl example

```sh
TOKEN="ons_pat_..."   # paste from the create response
API="https://app.onsager.ai"

curl -sS -H "Authorization: Bearer $TOKEN" \
  $API/api/sessions | jq

# Tenant-scoped PAT can post sessions in its own workspace:
curl -sS -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"prompt":"hello","project_id":"<scoped-project-id>"}' \
  $API/api/tasks
```

## Notes for v1

* No fine-grained scopes — every PAT carries `["*"]`. The schema reserves
  the column for a future scope language.
* Tokens are user-owned; tenant-owned / service-account principals are a
  separate spec.
* No IP allowlists, per-token rate limits, or rotation reminders.
* No CLI tooling that auto-stores the token; that's tracked as a follow-up.
