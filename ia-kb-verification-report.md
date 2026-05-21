# IA Redesign — KB Verification Report

Scope: read-only verification of 8 KB items to ground ADR drafting for the
pipeline-centric three-tab IA + chat-as-portable-component redesign. No
ADRs drafted here.

Inputs used:
- Onsager repo: `/home/user/onsager` (branch `claude/verify-onsager-ia-kb-sxbg9`)
- Notion KB Notes collection: `c996eeba-afed-4c51-a22b-31f2ecd6e40f`
- KB Review page: `361d7919-9ca6-81db-a2f1-f44094d8ba74` (confirmed —
  Notion MCP connector reachable; daily kb-triage routine page)
- Domain filter applied implicitly via search; relevant pages all carry
  `foundry/onsager`

---

## Blocking items

### T1. spec #289 / #398 status

- **spec #289** — "spec(dashboard): two-surface IA — Chat (R&D) + Workflows (production)"
  - state: `closed` / state_reason: `completed`
  - created: 2026-05-09; **last_modified: 2026-05-13T08:15:30Z**; closed: 2026-05-13 by tikazyq
  - labels: `spec`, `feat`, `priority:high`, `area:dashboard`, `in-progress`
  - `Superseded by`: **none declared** on the issue. The PR-history check
    list in the body shows PR 1 merged (PR #299) and PR 2a–6 still
    unchecked at close, so the spec was closed without all sub-PRs landing
    — the umbrella was retired rather than fully delivered.
  - URL: <https://github.com/onsager-ai/onsager/issues/289>

- **spec #398** — "[FTUE] Two-path entry — Cloud signup vs OSS install convergence to ChatPage"
  - state: `closed` / state_reason: `completed`
  - created: 2026-05-18; **last_modified: 2026-05-18T13:13:39Z**; closed: 2026-05-18 by tikazyq
  - labels: `spec`, `area:dashboard`, `ftue`
  - `Part of #397` (FTUE umbrella, also closed 2026-05-19)
  - `Superseded by`: **none declared**.
  - URL: <https://github.com/onsager-ai/onsager/issues/398>

- **KB sources** (Notion, foundry/onsager):
  - "Onsager 产品定位 v0.1" (Stable, 2026-05-18): references `spec #289`
    explicitly — *"Two-surface IA (`spec #289`): Sidebar 只剩 Chat +
    Workflows"*. URL: <https://www.notion.so/364d79199ca681b4953be66481634f97>
  - No Notion page found that re-litigates #289 or #398 after their
    close dates. The KB takes the two-surface IA as accepted background;
    it doesn't yet record any successor decision.

- **Decision**: `proceed-with-supersede`.
  Both issues are closed/completed, neither carries a `Superseded by`
  link, and the KB position quoted above treats #289 as the *current*
  IA frame — exactly what new ADRs 0019/0020 would change. Closing the
  umbrella while sub-PRs 2a–6 went un-shipped means the active IA in
  code is intermediate between #289's "two-surface" target and today's
  10-route reality, which is fine ground to land a new architectural
  decision on. No human-blocking ambiguity.

### T2. ADR numbering conflict

- **Next available ADR: `0019`** (highest existing = `0018-five-kernel-invariants.md`,
  Accepted 2026-05-15). `0020` is also free.
- **In-flight ADR 0019..0025 IA work**: none.
  - `mcp__github__search_issues` query for `ADR 0019 OR 0020 OR 0021 OR
    0022` in title/body, all states: **0 results**.
  - Notion search for "ADR 0019" / "ADR 0020": no hits.
  - The only KB mention of unwritten ADRs in that band is the *closed*
    plan from "DAG substrate 替代叙事" listing ADRs 0009–0018; that
    block has now landed (see ADR index lines 22–31 of
    `docs/adr/README.md`). Nothing reserved past 0018.
- **In-flight IA work in spec #44x band**:
  - The IA conversation lived in the FTUE umbrella `#397` and children
    `#398`–`#408` — all `closed/completed` by 2026-05-19.
  - Searches for "Workflows Runs Activity tab pipeline IA redesign" and
    for "universal chat entry portable component drawer" returned 0
    matching open or closed issues.
- **Conflict risk**: `none`. ADR 0019 + ADR 0020 are free and there is
  no parallel spec or ADR draft competing for the same surface.

### T3. ADR vs spec path

- **Frontend IA decisions go through: `ADR` (with a spec issue per ADR
  for tracking + implementation)**.
- Basis:
  - `docs/adr/README.md:3-5` — ADRs record "architectural decisions" and
    their dev-process counterpart (per ADR 0002 / now ADR 0013).
  - `docs/adr/README.md:33-54` — the "How to add an ADR" section
    prescribes a `Tracking issues` field linking to the spec and any
    sub-issues; ADRs and specs are paired, not alternatives.
  - **ADR 0008** (`docs/adr/0008-portal-owns-the-agent-control-plane.md`):
    portal / dashboard / route ownership is precisely the kind of
    decision ADRs already cover (`Tracking issues: #291`). The IA
    redesign — top-level navigation shape, chat surface ownership,
    workflow-detail collapse, vocabulary metaphor scope — is a
    direct cousin of ADR 0008's route-ownership rule and clearly inside
    ADR scope.
  - **ADR 0007** (`tools-and-skills-as-the-public-contract`) similarly
    binds a UX-facing protocol decision (MCP control plane) via ADR
    plus a tracking spec — same shape as the proposed 0019/0020.
- Recommendation: open ADR 0019 + ADR 0020 with one tracking spec issue
  each (the spec carries the Plan checklist; the ADR carries the
  decision rationale + dev-process counterpart). Both forms are
  required by the repo's existing convention; ADR alone or spec alone
  would deviate.

---

## Items shaping ADR content

### T4. Factory metaphor position

- **KB verbatim** (most authoritative single quote, "Onsager 产品定位 v0.1",
  Status: Stable, 2026-05-18,
  <https://www.notion.so/364d79199ca681b4953be66481634f97>):
  > **约束**: 隐喻只活在 user-facing copy / 教学 / landing page / ChatPage
  > 提示文案。**不进** IA / 路由 / 组件命名 / 代码 identifier / 数据库 schema。
  > 一旦用户做完第一个 workflow,技术词汇接管。

- **KB verbatim** (Onsager root note, Stable, 2026-05-17,
  <https://www.notion.so/35cd79199ca68131b4f6efac9ed8c3d6>):
  > Onsager 是 **AI-augmented work executor** — 把人+AI 在外部协作产出的
  > spec plan 可靠执行到 artifact / PR 的工厂级 substrate。「AI factory」
  > 旧定位 (2026-05-14 修正)：暗示 AI 自动管理一切，过头且易失败；新定位
  > 更诚实——AI 在分解和执行节点里都参与，但控制权在人。

- **KB verbatim** (FTUE umbrella spec #397, Locked context table):
  > Mental model metaphor — "AI Factory" — see vocabulary table in
  > `## Notes` below. The metaphor lives in **user-facing copy and
  > storytelling**, NOT in IA, route names, or component identifiers.

- **Agrees with tightening to marketing-only**: `partial — needs explicit
  amendment`.
  - KB already excludes IA / routes / identifiers / DB schema (strong
    alignment). KB *includes* `ChatPage 提示文案` and `教学` and
    `landing page` as legitimate surfaces.
  - The proposed move "marketing surfaces only, **all 5 dashboard
    locations removed**" is **strictly tighter** than the current KB
    position: it would *also* strip `ChatPage 提示文案` and any
    in-dashboard teaching copy. That's a defensible tightening, but it
    is not what KB currently says — flag as a deliberate amendment in
    the ADR rather than a restatement.
  - `[memory-drift]` — the task brief's memory ("confined to
    user-facing copy only") is *looser* than the KB (which also names
    routes/identifiers/schema as forbidden surfaces) and the *proposed
    tightening* is again moving in a different axis (location count,
    not just type). The ADR should explicitly enumerate the five
    locations being removed so the diff vs KB is auditable.

- **Citations**:
  - <https://www.notion.so/364d79199ca681b4953be66481634f97> (主条款)
  - <https://www.notion.so/35cd79199ca68131b4f6efac9ed8c3d6> (重定位历史)
  - <https://www.notion.so/360d79199ca6810fa8eed47f03498bf8> ("AI factory"
    暗示 AI 自动管理一切 — 修正动机)
  - `docs/adr/README.md:33-54` (ADR convention) + spec
    <https://github.com/onsager-ai/onsager/issues/397> (FTUE umbrella's
    factory-metaphor vocabulary table, the operational artifact)

### T5. Activation ladder framing

- **KB verbatim** ("Onsager 产品定位 v0.1", Stable,
  <https://www.notion.so/364d79199ca681b4953be66481634f97>):
  > # Activation Ladder (4 阶梯)
  > 1. **Inspected** — 落地后停留 >60s 或切换 DAG/YAML 视图
  > 2. **Drafted** — 产出 workflow draft (无需 workspace) ← 新关键 metric
  > 3. **Bound** — draft 绑到真实 repo (workspace + install + project 按需创建)
  > 4. **Activated** — 第一次 trigger 触发 + run 跑完
  >
  > `Drafted` 比 "workspace created" 更接近 "已采纳 abstraction" 的真实信号。
  > 当前 onboarding 把 `Bound` 当 activation, 跳过了 `Drafted` 阶段, 这是
  > FTUE 重设计要修的核心点。

- **KB verbatim** (FTUE umbrella spec #397):
  > `Drafted` is the new key indicator. It captures the moment a user
  > has invested cognitive effort designing their own workflow — well
  > before they create infrastructure.

- **Agrees with metric-not-IA framing**: **yes**.
  - The KB names the ladder explicitly as a **metric** ("新关键
    metric"; "key indicator"; "transition") — never as a navigation
    surface.
  - Spec #404 (closed, but the umbrella references it) treats the four
    rungs as `ftue.inspected/drafted/bound/activated` **events** in
    `activation_events` — instrumentation, not IA.
  - The proposed framing "ladder is a metric dimension, not an IA
    axis" matches both verbatim quotes. No tightening needed; the
    redesign just needs to make the ladder *stay* on the metrics axis
    and not creep back into the sidebar (e.g. as separate
    Drafted/Bound/Active/Paused top-level surfaces — which is exactly
    what the IA redesign rejects).
  - No `[memory-drift]`: memory and KB align here.

- **Citations**:
  - <https://www.notion.so/364d79199ca681b4953be66481634f97> (canonical
    4-rung definition + Drafted-as-metric framing)
  - <https://github.com/onsager-ai/onsager/issues/397> (umbrella's
    "Activation ladder" section, identical four-rung list, used as
    spec-axis locator — not as IA)
  - <https://github.com/onsager-ai/onsager/issues/398> (#398 cites the
    ladder as a *transition* to improve, not a surface)

### T6. MCP stance references

- **KB verbatim** (Onsager root note, Stable, 2026-05-17,
  <https://www.notion.so/35cd79199ca68131b4f6efac9ed8c3d6>):
  > **MCP 控制面**：Onsager substrate UI-portable via MCP control plane
  > (intent / spec_plan / compile / run / status / artifacts / verdicts);
  > Claude / Cursor / Onsager UI 互换 intent frontends; 原则：best UX per
  > surface，不为 MCP 提前约束架构 (SDD reflex)。MCP Apps SEP-1865
  > (2026-01-26 GA, ui:// + sandboxed iframe + postMessage 之上跑
  > JSON-RPC)：Stiglab dashboard + Forge/Synodic trace = MCP App
  > 候选位…

- **KB verbatim** ("MCP Apps (SEP-1865)",
  <https://www.notion.so/361d79199ca6815d8398cc509d7fe01b>):
  > **Onsager — Forge artifact 状态机、Synodic gate verdict trace 可视化。**
  > 同 Stiglab,把 spine 状态、governance 决策路径以 MCP App 形式暴露
  > 给 dev workflow 内的 agent session 提供 inline inspection
  >
  > ## 待决
  > - [ ] Onsager Forge/Synodic 的 trace 可视化是否走 MCP App 而非独立 web UI

- **Relevant KB entries** (URLs):
  - <https://www.notion.so/35cd79199ca68131b4f6efac9ed8c3d6> — "Onsager"
    root note: MCP control plane as the portable interop layer; UI is
    one of N intent frontends. **Strongly supports chat-as-portable**.
  - <https://www.notion.so/361d79199ca6815d8398cc509d7fe01b> — "MCP
    Apps (SEP-1865)": Forge/Synodic trace as MCP App candidate; cites
    dashboard as a *possible* MCP App rendering. Not yet decided.
  - <https://www.notion.so/360d79199ca6810fa8eed47f03498bf8> — "DAG
    substrate 替代叙事": frames Onsager as substrate with MCP control
    plane as a primary contract.

- **Recommended `depends_on` (Tracking issues + Supersedes lineage) for
  ADR 0020 (chat-as-portable-component)**:
  - **ADR 0007** — `tools-and-skills-as-the-public-contract`. Primary.
    Names the MCP backend + skills bundle as the contract that
    "Claude / Cursor / Onsager UI" all consume. Chat-as-portable is a
    direct consequence: the dashboard chat is one MCP client among
    many.
  - **ADR 0008** — `portal-owns-the-agent-control-plane`. Secondary.
    Portal owns the public surface; the chat's MCP endpoints live
    behind that boundary. Any "portable mount mode" decision lands
    inside portal's control-plane scope.
  - **ADR 0009** — `three-layer-pipeline`. Reference-only. The
    chat-as-portable surface is the user side of the
    `Intent → Spec Plan` boundary that ADR 0009 names. Cite for
    context, do not list as a hard dependency.
  - **(Not yet ADR'd) MCP Apps SEP-1865 / ui://**. The Notion page is
    `Inbox` status with an open `[ ]` "待决" item asking whether
    Forge/Synodic trace renders as MCP App vs independent web UI.
    Recommend the new ADR 0020 either (a) defers this decision
    explicitly and references the KB page as forward link, or (b)
    names a follow-up spec.

- **Recommended `depends_on` for ADR 0019 (pipeline-centric three-tab
  IA)**:
  - **ADR 0006** — `edge-dispatcher-as-the-public-boundary` (route
    ownership cousin).
  - **ADR 0008** — `portal-owns-the-agent-control-plane` (same).
  - **ADR 0009** — `three-layer-pipeline`. The "Workflows / Runs /
    Activity" tab triple is the user-facing projection of the
    Spec Plan → Workflow → Execution Plan pipeline ADR 0009 defines.
    Hard dependency.
  - **ADR 0015** + **ADR 0016** — Spec Plan DAG external contract +
    Workflow Library N isomorphic islands. The Workflows tab's content
    model is exactly the Workflow Library; cite for content shape.
  - Spec #289 (closed) and spec #397 (closed) as the historical
    `Supersedes` lineage in human prose; do not list under `Supersedes:`
    in the ADR frontmatter because #289/#397 are *specs*, not ADRs (the
    repo convention reserves `Supersedes` for ADR-to-ADR replacement;
    spec relationships go in body prose or in the spec issue's own
    fields).

---

## Parallel items

### T7. spec #406

- **Summary**: `[FTUE] Template library v0 — initial 5–6 templates
  dominantly class D`. Part of FTUE umbrella #397. Defines six static
  templates rendered as a gallery in ChatPage's empty state (auto-merge
  / spec-compliance-gate / labeled-issue-summary /
  release-notes-from-merged-prs / security-review-on-pr /
  onsager-dogfood). Five class-D (Quality Gate) + one signature
  Dogfood. Templates ship as JSON at
  `apps/dashboard/src/lib/templates/v0.json`. Status: `closed` /
  `completed` (2026-05-19).
- Repo evidence (live code):
  - `apps/dashboard/src/components/chat/TemplateGallery.tsx:10`
  - `apps/dashboard/src/pages/ChatPage.tsx:559`
  - `apps/dashboard/tests/smoke/ftue-templates.test.tsx:47,64,113`
- KB: no dedicated Notion page for #406, but the locked context for the
  template set lives in FTUE umbrella #397 + "Onsager 产品定位 v0.1".
- URL: <https://github.com/onsager-ai/onsager/issues/406>

- **Relation to IA redesign**: `related, but not blocking`.
  - #406 lives **inside** ChatPage — i.e. inside the surface the IA
    redesign is reshaping into a portable global component. Anywhere
    the chat surface mounts (FAB / Sidebar / Drawer / Palette), the
    template gallery must continue to render. The new ADR 0020 should
    cite #406 in passing to confirm the empty-state UI travels with
    chat, not with a specific mount mode.
  - #406 does NOT depend on the new 3-tab IA decision, and the new ADR
    0019 does not need to re-litigate template-library shape.
  - #406's `factory_framing` field is one of the named *user-facing*
    metaphor surfaces (e.g. *"A QC checkpoint that lets clean PRs ship
    themselves."*). When ADR 0019 enumerates the five dashboard
    locations being removed (T4), it must distinguish between template
    `factory_framing` strings (which live in JSON content, user-facing,
    and the FTUE umbrella locks them) and any structural factory copy
    in the IA chrome. KB and #397 currently treat
    `factory_framing` as legitimate user-facing copy.

### T8. ADR convention

- **Frontmatter fields** (canonical order, per
  `docs/adr/README.md:36-46` + observed in ADRs 0005–0018):
  1. `Status` — `Accepted` / `Proposed` / `Superseded`. Free-form
     parenthetical for amendments (e.g. "Accepted (partial-keep,
     2026-05-15)").
  2. `Date` — ISO date (creation; later acceptance / amendment dates
     may follow in parens).
  3. `Identity impact` — exact value `yes` or `no`. Mandatory from
     ADR 0005 onward. Mechanically scannable.
  4. `Tracking issues` — links to the parent spec and any sub-issues.
  5. `Supersedes` — prior ADR number(s), or `none`. **ADR-to-ADR
     only**; spec lineage goes in body prose.
  6. `Superseded by` — `none` at creation; updated if a later ADR
     replaces this one.

- **Example frontmatter block ready to paste** (for ADR 0019; tweak for
  0020):

```markdown
# ADR 0019 — Dashboard IA: pipeline-centric three-tab surface

- **Status**: Proposed
- **Date**: 2026-05-21
- **Identity impact**: no
- **Tracking issues**: #<NEW-SPEC> (implementation spec). Refines the
  IA frame established by spec #289 (closed without all sub-PRs
  landing) and the FTUE umbrella #397.
- **Supersedes**: none
- **Superseded by**: none
```

  For ADR 0020 the `Tracking issues` line should additionally reference
  ADR 0007 (MCP public contract) and ADR 0008 (portal owns the control
  plane) — these are *dependencies* expressed in prose, not in a
  separate `depends_on:` field (the repo does not use one — see drift
  note below).

- **Body sections** (canonical order, per `docs/adr/README.md:48-50`):
  Context → Decision → Rejected alternatives → Consequences (positive /
  negative-trade-offs / neutral) → **Dev-process counterpart** (per
  ADR 0002 / now ADR 0013) → Adoption checklist → Out of scope.

- **Other conventions** (observed across 0001–0018):
  - First-level heading is `# ADR NNNN — <slug title>`. Use an em-dash
    between number and title.
  - No section numbering. Distinct H2 titles only.
  - No time anchors in titles (titles describe the *decision*, not the
    date — date lives in the metadata block).
  - No ceremony adjectives ("important", "critical", "comprehensive")
    in titles.
  - Rejected alternatives list is required, not optional. Each
    alternative gets a short paragraph with the reason for rejection.
  - The Adoption checklist uses `- [ ]` items so PRs can tick them off
    as the implementation lands; flipping the ADR's Status to
    `Accepted` is itself the last checklist item.
  - Cross-ADR references use the ADR number, not a markdown link, when
    referring to *concepts* (e.g. "per ADR 0009"); markdown links are
    used only in the metadata block and explicit "see ADR NNNN" call-outs.
  - **`[memory-drift]`** — the task brief's memory says "minimal
    frontmatter (`depends_on` + `identity_impact` only); git owns
    status/version/dates/tags". That does **not** match the repo. The
    repo's convention (read directly from `docs/adr/README.md` and the
    18 existing ADRs) requires six fields: `Status`, `Date`,
    `Identity impact`, `Tracking issues`, `Supersedes`,
    `Superseded by`. There is no `depends_on:` field in the repo. The
    ADRs do carry status and date in-file (git is *not* the sole owner
    of those). When drafting 0019/0020, follow the **repo convention**,
    not memory.

---

## Next-step recommendation

Based on the findings above:

- **Draft ADR 0019 (pipeline-centric three-tab IA)**: `go`.
  - Numbering free (T2). Path is correct (T3). Activation-ladder framing
    aligned with KB (T5). Predecessor spec #289 is closed without a
    declared successor (T1), so the new ADR is the right venue. The
    factory-metaphor tightening (T4) is the only spot where the ADR is
    making a *new* commitment beyond KB; the ADR should enumerate the
    five dashboard locations being removed and state explicitly that
    this tightens but does not contradict the KB position.
  - Hard `depends_on` (prose, since no field): ADR 0009 (pipeline →
    Workflows/Runs/Activity projection); ADR 0015 + ADR 0016 (Workflow
    library content shape).
- **Draft ADR 0020 (chat as portable global component with contextual
  auto-scope)**: `go`.
  - Numbering free. Strongly supported by KB's MCP-control-plane
    framing (T6). Predecessor spec #289's chat-surface section is
    superseded by reshaping chat into N mount modes — same surface,
    different chrome. ChatStorageKey schema change `(user_id,
    workspace_id) → (user_id, scope_type, scope_id)` is **not** in KB
    (`not in KB`) — this is a new schema commitment; flag it loudly in
    the ADR's Decision section and confirm with the owner.
  - Hard `depends_on` (prose): ADR 0007 (MCP as public contract);
    ADR 0008 (portal owns the agent control plane). Optional context:
    ADR 0009 (intent → spec plan boundary).
- **Switch to spec flow instead**: `no`. The IA + chat-portability moves
  are architectural (route ownership, public-surface shape, schema
  change). Per T3, ADRs are required for changes of this shape; specs
  alone would deviate from the repo convention.

- **Items needing human decision**:
  1. The exact list of "5 dashboard locations" where the factory
     metaphor is being removed (T4). KB constrains the metaphor by
     *type* of surface (no IA/routes/identifiers/schema) but doesn't
     enumerate locations. ADR 0019 needs the owner to confirm the five.
  2. Whether `factory_framing` strings inside `templates/v0.json`
     (spec #406) count as "user-facing copy" (KB says yes) or as a
     "dashboard internal" being removed (T4 proposal — unclear).
     Resolve before writing ADR 0019's "five locations" list.
  3. ChatStorageKey schema change `(user_id, workspace_id) →
     (user_id, scope_type, scope_id)` (T6 / Mission context) has no
     KB precedent. Confirm storage backend (localStorage today per
     #289; server-side is a follow-up) and decide whether the schema
     change is a same-PR data migration or a fresh-start (pre-launch
     posture permits the latter).
  4. The 4 mount modes (FAB / Sidebar / Drawer / Palette) and "default
     = Drawer" are **not in KB**. Owner ratification needed before ADR
     0020 hard-locks them. KB only supports "chat-as-portable-component"
     at the *concept* level (UI is one of N intent frontends via MCP).
  5. Whether ADR 0019 also formally retires spec #289 / #397 / #398
     (mark them `Closed by ADR 0019` in a follow-up comment) — these
     are closed but not annotated with the successor.

- **KB entries flagged for update** (out of scope for this task — just
  flag):
  - "Onsager 产品定位 v0.1"
    (<https://www.notion.so/364d79199ca681b4953be66481634f97>): the
    "前置上下文" section cites *"Two-surface IA (`spec #289`):
    Sidebar 只剩 Chat + Workflows"* as current state. Once ADR 0019
    lands, this line is stale and should be re-pointed to ADR 0019
    (three-tab IA) and ADR 0020 (chat as portable).
  - "MCP Apps (SEP-1865)"
    (<https://www.notion.so/361d79199ca6815d8398cc509d7fe01b>):
    Status `Inbox`. The open `[ ]` item *"Onsager Forge/Synodic 的
    trace 可视化是否走 MCP App 而非独立 web UI"* sits adjacent to the
    chat-as-portable decision; consider promoting or closing once
    ADR 0020 lands.
  - "Onsager" root note
    (<https://www.notion.so/35cd79199ca68131b4f6efac9ed8c3d6>): the
    Timeline is append-only; add an entry once ADR 0019/0020 land so
    the compiled-truth and timeline stay in sync.
  - "🔍 KB Review"
    (<https://www.notion.so/361d79199ca681dba2f1f44094d8ba74>): the
    KB review collection ID provided in the task brief
    (`c996eeba-afed-4c51-a22b-31f2ecd6e40f`) is the **Notes data
    source** parent, *not* this review page. Both are reachable, but
    if the brief intended the Review page as the canonical inventory
    of pending IA decisions, it does not yet contain a triage row
    for the new ADRs — a future kb-triage routine pass would surface
    them.

---

*Report generated 2026-05-21. All quoted KB content is verbatim from
the Notion pages cited; any `[unverified]` or `not in KB` annotations
mark items that could not be grounded against the KB or repo within
this verification pass. No KB writes were made.*
