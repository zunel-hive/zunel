# Profile-as-MCP-Server: Mode 2 — Agent-Loop-as-Tool

**Status:** Slice 1 shipped — `zunel mcp agent --mode2` registers the
`helper_ask` tool with caller-controlled session ids namespaced by API-key
fingerprint, `reject` (default) and `allow_all` approval policies, and a
CLI iteration ceiling. SSE streaming, approval forwarding, and per-call
cancellation remain v3 / future work as called out in the open questions
below. This doc captures both the product reasoning and what's actually
live today.
**Owner:** runtime / MCP.
**Builds on:** [`profile-as-mcp.md`](./profile-as-mcp.md) (Mode 1) and the
existing `AgentLoop` / `SessionManager` / `ApprovalHandler` machinery in
`zunel-core`.

## Motivation

Mode 1 lets a hub agent call helper-profile **tools**. The hub is still the
brain: it sees `mcp_research_read_file`, `mcp_research_grep`, etc. and drives
its own LLM loop to use them. That's the right primitive for "borrow a
helper's filesystem / OAuth tokens / auth scopes," but it puts the entire
LLM-orchestration burden on the hub. Three real workflows fall outside it:

1. **"Ask the research helper to draft a literature review"** — the helper
   should run its own loop with its own model, tools, and prompt; the hub
   just wants the answer.
2. **Specialization by profile** — different profiles legitimately want
   different system prompts, model picks, max-iteration ceilings, and
   approval policies. Mode 1 erases that by making the hub drive the loop.
3. **Auditability of cross-agent work** — when the hub sends a request to
   the helper, the helper's own session log is the natural place to record
   what happened. Mode 1 leaves no trace beyond raw tool calls.

Mode 2 introduces a single tool, `helper_ask`, that runs a full `AgentLoop`
inside the helper profile and returns the answer. Conceptually it's "RPC
over MCP into another agent."

## What slice 1 ships

| Surface | State |
|---------|-------|
| `--mode2` flag | **Implemented.** Off by default; opting in registers `helper_ask` alongside Mode 1's filtered registry. |
| `helper_ask` tool | **Implemented.** Single-JSON response, `prompt` / `session_id` / `max_iterations` args. |
| Caller-controlled session id | **Implemented.** Namespaced as `mode2:<caller_fingerprint>:<caller-supplied or fresh>`. Loopback-no-auth callers fall back to `anon`. |
| `_meta` side-channel | **Implemented.** Returns `session_id`, `tools_used`, and `usage` (mapped to MCP's `_meta` convention). |
| `--mode2-approval reject\|allow_all` | **Implemented.** `reject` is the default; `forward` is deferred. |
| `--mode2-max-iterations` | **Implemented.** Hard ceiling, `min()`-ed against caller's `max_iterations` arg and the helper's `agents.defaults.max_tool_iterations`. |
| SSE streaming | **Deferred.** Slice 2. |
| Per-call cancellation | **Deferred.** Slice 2 (needs `AgentLoop::process_streamed` to take a `CancellationToken`). |
| Approval forwarding | **Deferred to v3.** |

## Open product questions

These are why Mode 2 didn't ship the entire surface in one go. Each is a
real decision; we list a recommendation, the actual choice slice 1 made,
and (for deferred items) what would unblock the next slice.

### 1. Streaming

`AgentLoop::process_streamed` already produces token deltas, tool-call
events, and a final aggregated `RunResult`. MCP supports two response
shapes for `tools/call`:

- **Single JSON.** `Content-Type: application/json` — works today, blocks
  until the helper finishes, returns one `result.content` array.
- **Server-Sent Events.** `Content-Type: text/event-stream` — supported by
  Mode 1's transport already; we'd emit `progress` notifications mid-call
  and a final `result` SSE.

**Recommendation.** Default v2 to single-JSON for simplicity. Layer SSE
streaming behind an `Mcp-Stream: deltas` request header (or a `stream:
true` field in the tool args, if MCP ergonomics prefer that). The transport
plumbing already exists; the work is teaching `AgentLoop` to forward
deltas to a caller-supplied sink instead of (or alongside) the local
`MessageBus`.

**Why default off:** most helper calls are batch ("draft this"), not
chat-style (where you'd watch tokens stream). Streaming is a
nice-to-have for v2; not blocking initial release.

### 2. Approvals

The helper profile may have `approval_required = true` for tools the hub's
prompt indirectly causes. Two reasonable policies:

- **`reject`** — any tool that needs approval inside `helper_ask` causes
  the call to fail with a structured error. Simple, audit-friendly, but
  means the hub can't drive a long helper task that needs even one
  approval mid-loop.
- **`forward`** — bridge `BusApprovalHandler` over MCP via a callback
  side-channel (helper sends an `approval_request` notification, hub
  must respond). Maximally flexible but introduces a stateful
  bidirectional channel that MCP doesn't really model today.

**Recommendation.** v2 ships **`reject`** as the default and adds an
explicit `--mode2-approval=allow_all` opt-in for trusted operators (e.g.,
"I'm running a fully read-only helper, just allow everything"). `forward`
is a v3 extension. The CLI flag is intentionally narrow and explicit so
operators can't enable allow-all by accident.

### 3. Cancellation

The hub may abort a helper call (user cancels, deadline expires). MCP has
`notifications/cancelled` for this. Internally `AgentLoop::process_streamed`
already takes a `CancellationToken` (or equivalent — verify before
implementing). The helper transport must:

- map an MCP cancel notification to the in-flight call's token
- guarantee the in-progress LLM request is aborted (provider-specific —
  most reqwest/streaming HTTP clients support drop-to-cancel)
- guarantee any tool calls in-flight at cancel time finish or are aborted
  cleanly per the existing `Tool::cancel` contract (today: best-effort)

**Recommendation.** Implement cancellation in v2 from day one — partially
because the existing graceful-shutdown work (`http::run`'s
`CancellationToken`) makes it cheap, mostly because a runaway helper that
ignores cancellation is a much louder regression than a missing feature.

### 4. Session persistence

Each `helper_ask` call is conceptually a "turn" in some session. Three
options:

- **Stateless.** No session is created; each call is independent. Mode 1's
  current behavior. Loses cross-call memory; doesn't pollute the helper's
  session log.
- **One session per hub profile.** The session key is derived from the
  caller's identity (e.g., the API key used to authenticate). The helper
  has one persistent thread per upstream consumer. Good audit, but
  identity → session mapping is a new concept.
- **Caller-controlled.** Add a `session_id` field to the tool args. Hub
  decides; helper just respects it. Simple, but two unrelated hub calls
  can clobber each other if they pick the same key.

**Recommendation.** v2 makes the session **caller-controlled with a default
of fresh-per-call**:

- `helper_ask({prompt, ...})` → starts a new session, returns its id
  alongside the result.
- `helper_ask({prompt, session_id: "..."})` → resumes that session.
- The session id is namespaced (`mode2:<api_key_fingerprint>:<caller-supplied>`)
  so two unrelated hubs can't collide even if they pick the same id.

This mirrors the `--session` flag on `zunel agent` and avoids new identity
concepts. The fingerprint is the constant-time-comparison hash of the API
key already used for auth.

### 5. Per-call iteration ceilings

The helper has its own `agents.defaults.max_iterations`. The hub may want
to bound a single `helper_ask` call further (e.g., "spend at most 6
turns, then give up"). Two surfaces:

- **Tool arg.** `helper_ask({prompt, max_iterations: 6})`. Caller sets
  the ceiling explicitly.
- **CLI cap.** `zunel mcp agent --mode2-max-iterations 6` sets a hard
  upper bound the caller cannot exceed.

**Recommendation.** Both. Tool arg defaults to the helper's
`agents.defaults.max_iterations`; CLI cap is a hard ceiling that the tool
arg is `min()`-ed against. Mirrors how Linux ulimits work.

### 6. Token / cost accounting

Helper calls run on the helper's billing identity (its provider, its
account, its quotas). Two things matter:

- **Caller visibility.** The result's `_meta` field (an MCP convention)
  carries the helper's `RunResult.usage` so the hub knows what the call
  cost. v2 ships this.
- **Helper-side budgeting.** A `--mode2-monthly-token-budget` cap that
  refuses new `helper_ask` calls once exceeded. **Deferred to v3** —
  needs persistent counters that the codebase doesn't currently have.

### 7. Tool surface inside the helper

When `helper_ask` runs inside the helper profile, what tools does the
helper see during that loop?

- **Helper's full registry, modulo Mode 1's gates.** I.e., the same
  filtered registry Mode 1 exposes (still gated by `--allow-write`,
  `--allow-exec`, `--allow-web` on the agent CLI).
- **Or a different gate set specifically for Mode 2.**

**Recommendation.** Reuse the existing Mode-1 gates verbatim. A helper
configured "read-only over MCP" in Mode 1 should *also* be read-only when
talking to itself via `helper_ask`. This avoids a second mental model
operators have to track. If a deployment wants Mode 2 to write but Mode 1
not to, run two agent processes — they're cheap.

## Proposed v2 surface

### CLI

```text
zunel [--profile NAME] mcp agent [TRANSPORT] [AUTH] [DEPTH] [LIMITS] [TOOL GATES] [MODE 2]

MODE 2 (all opt-in)
  --mode2                          Enable the helper_ask tool. Off by default
                                   (Mode 1 stays the only registered surface).
  --mode2-approval <policy>        reject (default) | allow_all. Forwarding
                                   approval requests over MCP is v3.
  --mode2-max-iterations N         Hard upper bound on a single helper_ask
                                   loop. Defaults to the helper's own
                                   agents.defaults.max_iterations.
  --mode2-stream                   Enable SSE streaming for helper_ask
                                   responses. Off by default.
```

When `--mode2` is set, the agent registers exactly one extra tool —
`helper_ask` — alongside Mode 1's filtered registry. They coexist; you
can have a hub that uses both `mcp_helper_read_file` (Mode 1) and
`mcp_helper_helper_ask` (Mode 2) on the same helper.

### Tool schema

```json
{
  "name": "helper_ask",
  "description": "Ask the helper agent to handle a prompt with its own LLM and tool registry.",
  "inputSchema": {
    "type": "object",
    "required": ["prompt"],
    "properties": {
      "prompt":         { "type": "string" },
      "session_id":     { "type": "string", "description": "Caller-supplied session key. Omit for a fresh session." },
      "max_iterations": { "type": "integer", "minimum": 1 },
      "system_prompt":  { "type": "string", "description": "Per-call system-prompt override. Subject to helper-side allowlists." }
    }
  }
}
```

### Result shape

```jsonc
{
  "content": [
    { "type": "text", "text": "<helper's final answer>" }
  ],
  "isError": false,
  "_meta": {
    "session_id": "mode2:abc123:research-2026-04-26",
    "iterations": 3,
    "tools_used": ["read_file", "grep"],
    "usage": { "input": 1234, "output": 456, "reasoning": 0, "cached": 100 }
  }
}
```

`_meta` is the MCP convention for non-content metadata; clients that
don't recognize it ignore it without breaking. The `usage` block matches
`zunel_providers::Usage` so it round-trips through existing serializers.

### Dispatcher

A new `Mode2Dispatcher` in `zunel-cli/src/commands/mcp/` wraps:

- the same `RegistryDispatcher` Mode 1 uses (so non-`helper_ask` calls
  still work)
- a single `helper_ask` handler that constructs an `AgentLoop` per call,
  resolves the session via `SessionManager`, runs `process_streamed`, and
  formats the result

The dispatcher is `Arc`'d and shared across requests, but each
`helper_ask` invocation gets its own `AgentLoop` instance to avoid
accidental cross-request state.

## Interaction with existing v1 features

| v1 feature                | Mode 2 behavior |
|---------------------------|-----------------|
| `Mcp-Call-Depth` cap      | `helper_ask` itself counts as one hop. Tools the helper invokes inside its loop continue to forward depth via the existing `ToolContext::incoming_call_depth` plumbing, so a depth-cap of 8 can still bound A → B → C → … chains. |
| Origin allowlist          | Unchanged. Mode 2 is just one more tool in the registry from the transport's POV. |
| `--max-body-bytes`        | Bounds the prompt + args size; nothing Mode-2-specific. Recommend documentation noting that long prompts may need a larger cap. |
| Graceful shutdown         | `helper_ask` calls in flight at SIGTERM time get the same 5-second drain window. The CancellationToken plumbed for shutdown is reusable for per-call cancellation (see Q3). |
| Tool gates                | `helper_ask` is gated behind `--mode2`, *not* behind `--allow-exec` or friends. The helper's *internal* tool registry inside the loop still respects the gates. |

## Non-goals for v2

These keep the diff bounded; each is a follow-up:

1. **Approval forwarding** (Q2's `forward` policy). Defer; v2 ships `reject`
   + `allow_all` only.
2. **Multi-helper composition** (helper A calls helper_ask on helper B).
   Technically it Just Works because the depth header forwards, but no
   product testing is planned for v2.
3. **Persistent quotas** (Q6's monthly cap). Needs new accounting
   infrastructure.
4. **Dynamic system prompt** beyond the `system_prompt` arg (Q schema
   above). Helper-side allowlist of permitted overrides is v3.
5. **Bidirectional sessions** where the helper can ask the hub follow-up
   questions. v2 is request-reply only.
6. **Cross-process session sharing.** Each helper still has its own
   `SessionManager` rooted in its own `ZUNEL_HOME`. Two helper processes
   on the same profile would race; v2 keeps the "one server per profile
   per host" rule from Mode 1.

## Risks worth flagging up front

- **Runaway loops.** A `helper_ask` call with a model that gets stuck in
  a tool-call loop can burn tokens quickly. Mitigation: hard ceiling via
  `--mode2-max-iterations`, plus the existing per-iteration timeouts in
  `AgentLoop`. Recommend documenting "this is a budget multiplier" in
  bold.
- **Prompt injection across hops.** Mode 2 lets a prompt from one agent
  drive another agent's tools (within the helper's gate set). The gate
  system already protects against catastrophic actions, but operators
  should treat helper APIs as untrusted input. Document this prominently.
- **Approval UX.** A reject-only default is correct for safety but
  operators will hit it. Make sure the error message clearly says
  "this helper requires approval for tool X; either run with `--mode2-approval
  allow_all` or remove approval requirements for that tool."

## Open questions — closed for slice 1

1. ~~Does `AgentLoop::process_streamed` already accept a `CancellationToken`?~~
   **Closed.** It does not. Slice 1 leans on the iteration cap +
   request-body cap for control. Slice 2 is responsible for the
   prerequisite refactor.
2. ~~Where does the API-key fingerprint live?~~ **Closed.**
   `DispatchMeta::caller_fingerprint` is populated by the HTTP transport
   from the same `token_fingerprint` helper the access log uses, then
   stamped onto `ToolContext::caller_fingerprint` by
   `RegistryDispatcher`. `helper_ask` reads it from the context.
3. **Still open.** Streaming spec: SSE-with-`progress`-notifications
   (closer to the MCP spec's intent) vs. a custom `text/event-stream`
   shape. Slice 1 ships single-JSON only; slice 2 picks the wire format.
4. **Still open.** How do we surface `helper_ask`'s `_meta.usage` in
   the hub's local token accounting? Slice 1 returns it on the wire so
   downstream tooling has the data; aggregation belongs to a future
   reporting change.

## Decision log

- **Default off.** v2 ships with `--mode2` opt-in. Mode 1 remains the
  only-on-by-default surface. Rationale: agent-loop-as-tool is a
  meaningfully bigger trust delegation than tool-passthrough, so the
  operator must consciously turn it on.
- **One tool, not two.** We considered exposing both `helper_ask` and a
  separate `helper_followup({session_id, prompt})`. Chose to fold both
  into `helper_ask` with an optional `session_id` to keep the surface
  area minimal.
- **No streaming in v2 default.** The transport supports it; the
  ergonomics are uncertain; defer until we have a real client driving
  it. If a v2 user complains, flip the default — easy rollback.
