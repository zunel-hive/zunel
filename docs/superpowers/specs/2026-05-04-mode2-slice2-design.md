# Mode 2 Slice 2 — design spec

**Status:** Approved 2026-05-04. Ready for implementation plan.

**Companion docs:**
- [`docs/profile-as-mcp-mode2.md`](../../profile-as-mcp-mode2.md) — slice-1 doc that
  defines what's already shipped and which open questions slice 2 is asked
  to close.
- [`docs/profile-as-mcp.md`](../../profile-as-mcp.md) — Mode 1 (filtered tool
  registry passthrough) which Mode 2 layers on top of.

## Goal

Close the four deferred items from Mode 2 slice 1:

1. **`system_prompt` override** — caller-supplied per-call persona for the
   helper's `AgentLoop`.
2. **Per-call cancellation** — hub aborts a running `helper_ask` via MCP
   `notifications/cancelled`; the helper's loop tears down cleanly.
3. **SSE streaming** — the helper streams agent-loop progress back to the
   caller as the turn runs, instead of buffering until completion.
4. **Approval forwarding** — when the helper's loop hits a tool that needs
   approval, the hub can answer the approval request rather than the call
   failing.

Slice 1 already shipped:
- The `--mode2` flag, `helper_ask` tool, single-JSON response.
- Caller-controlled session ids namespaced by API-key fingerprint.
- `--mode2-approval reject|allow_all` and `--mode2-max-iterations` ceilings.

The slice-2 surface is **fully additive**: nothing about the slice-1 wire
format or CLI breaks. A hub that doesn't opt in to streaming, cancellation,
or `system_prompt` continues to see exactly the slice-1 behavior.

## Non-goals

These remain explicitly deferred (to slice 3 or later):

- Approval forwarding via callback URL or bidirectional MCP GET / SSE
  channels. Slice 2 ships the simplest forwarding path that fits the
  existing one-way HTTP transport: polling tools.
- `system_prompt` allowlist. Slice 2 enforces a length cap and an opt-out
  flag; per-call template restrictions are slice-3 work.
- Persistent token / cost budgets across calls.
- Multi-helper composition tests (helper A invokes `helper_ask` on helper B).
  Cancellation forwarding through that chain is out of scope.
- Cross-process session sharing. The "one server per profile per host"
  rule from slice 1 still applies.

## Architecture

### 1. `system_prompt` override

**Caller surface** — `helper_ask` accepts an optional `system_prompt`
string in its tool args:

```jsonc
{
  "name": "helper_ask",
  "inputSchema": {
    "type": "object",
    "required": ["prompt"],
    "properties": {
      "prompt":         { "type": "string" },
      "session_id":     { "type": "string" },
      "max_iterations": { "type": "integer", "minimum": 1 },
      "system_prompt":  { "type": "string", "description": "Per-call persona for the helper's AgentLoop. Limited to 8 KiB." }
    }
  }
}
```

**Plumbing** — extend `AgentLoop` with a builder method that owns an
optional extra system message, prepended *before* the skills system
message. The skills message is contextual ("you have these skills"); the
caller's `system_prompt` is operator persona ("you are a research
helper"). Stack order from server-side perspective:

1. Operator persona (`system_prompt` arg, when present)
2. Skills summary (existing `build_skills_system_message` output)
3. Persisted history (user/assistant turns)
4. Current user message

```rust
// zunel-core/src/agent_loop.rs
impl AgentLoop {
    pub fn with_extra_system_message(mut self, msg: Option<String>) -> Self {
        self.extra_system_message = msg;
        self
    }
}

// process_streamed_with_approval, where the skills message is currently
// inserted at index 0:
if let Some(extra) = self.extra_system_message.as_ref() {
    initial_messages.insert(0, ChatMessage::system(extra));
}
if let Some(skills) = self.build_skills_system_message() {
    let pos = if self.extra_system_message.is_some() { 1 } else { 0 };
    initial_messages.insert(pos, skills);
}
```

The extra message is **not persisted** to the session (same treatment as
the skills message) — the caller can change `system_prompt` between
calls on the same session and the new value applies cleanly.

**Operator controls**:

- `--mode2-disable-system-prompt` — when set, `helper_ask` rejects calls
  that pass a non-empty `system_prompt` with an `is_error` ToolResult.
- 8 KiB length cap enforced inside `HelperAskTool::execute`. Calls with
  longer values are rejected.

### 2. Per-call cancellation

**`AgentLoop` change** — add `process_streamed_with_cancel` alongside the
existing `process_streamed`:

```rust
pub async fn process_streamed_with_cancel(
    &self,
    session_key: &str,
    message: &str,
    sink: mpsc::Sender<StreamEvent>,
    cancel: tokio_util::sync::CancellationToken,
) -> Result<RunResult> { ... }
```

Implementation wraps the existing `process_streamed_with_approval` body in
`tokio::select!` against `cancel.cancelled()`. On cancel:

1. Try to persist whatever assistant messages have already landed in
   `result.messages` (best-effort; a partial save is better than none).
2. Return `Err(crate::error::AgentError::Cancelled)` (new variant).

The existing `process_streamed` becomes a thin shim that builds an
uncancelled token and delegates. Behavior for current callers
(`zunel-cli agent`, gateway path, all existing tests) is unchanged.

**Cancel registry on the dispatcher**:

```rust
// zunel-cli/src/commands/mcp/cancel_registry.rs
pub struct CancelRegistry {
    inner: Mutex<HashMap<RpcId, CancellationToken>>,
}
impl CancelRegistry {
    pub fn register(&self, id: RpcId) -> CancellationToken { ... }
    pub fn cancel(&self, id: &RpcId) -> bool { ... }   // true if found
    pub fn unregister(&self, id: &RpcId) { ... }
}
```

`RpcId` mirrors JSON-RPC's `id` shape: `Number | String`. We don't
support `null` ids — those are notifications and never get cancelled.

**Plumbing the request id to `helper_ask`**:

Today `DispatchMeta` carries `call_depth` and `caller_fingerprint`. Add
an `rpc_id: Option<RpcId>` field, populated by the HTTP transport from
the parsed JSON-RPC envelope. `RegistryDispatcher` stamps it on
`ToolContext::rpc_id`. `HelperAskTool::execute` reads it and registers
the cancel token.

The token is unregistered on every exit path via a guard struct (RAII).

**Routing `notifications/cancelled`**:

`RegistryDispatcher` (or a thin wrapper around it) recognizes the method
and calls `cancel_registry.cancel(params.requestId)`. Returns `None`
(notifications get 202 Accepted at the transport layer).

**Wall-clock cap** — `--mode2-call-timeout-secs N` (default 0 =
unbounded). When set, `HelperAskTool` spawns a watchdog that fires the
same cancel token after the deadline. Implemented as a `tokio::select!`
between `cancel.cancelled()` and `tokio::time::sleep(deadline)`.

### 3. SSE streaming

**Wire format** — MCP `notifications/progress` over `text/event-stream`,
exactly per the MCP spec's "Streamable HTTP" transport.

**Opt-in** — presence of `_meta.progressToken` in the inbound `tools/call`
request. No new CLI flag; the server always supports streaming, the
client decides whether to opt in. (Doc's `--mode2-stream` proposal was
unnecessary once we use the spec-defined token; it would be a
belt-and-suspenders kill-switch and YAGNI.)

**Event mapping**:

| `StreamEvent` variant | SSE frame |
|---|---|
| `ContentDelta(text)` | `notifications/progress { progressToken, progress: i++, message: text }` |
| `ToolCallDelta { name: Some(n), .. }` | `notifications/progress { progressToken, progress: i++, message: format!("[tool] {n}") }` |
| `ToolCallDelta { name: None, .. }` | not emitted (argument fragments are noisy and uninterpretable mid-stream) |
| `Done(_)` | not emitted as progress; the final SSE result frame conveys completion |

The progress sink is bounded (channel size 64); a slow client backs the
agent loop up at most one channel-buffer of deltas before the helper
blocks. Acceptable — keeps memory bounded and is a strong enough
backpressure signal for a misbehaving client.

**Transport changes** (`zunel-mcp-self/src/http.rs`):

- New `McpDispatcher::dispatch_streaming(message, meta, progress_sink) ->
  Option<Value>` method, default-implemented to delegate to `dispatch`
  and ignore the sink (Mode 1 servers and stdio dispatcher get this for
  free).
- `handle_post` opens a streaming code path when:
  1. The method is `tools/call`,
  2. The parsed body has a `params._meta.progressToken`, and
  3. The Accept header includes `text/event-stream`.

  Otherwise it falls through to the existing single-JSON / single-SSE
  path.

- New `write_streaming_response` writer:
  1. Writes the SSE response header (matches existing `write_sse_response`).
  2. For each progress event from the sink, writes one
     `event: message\ndata: { ...progress notification... }\n\n` chunk
     and flushes the stream.
  3. After the dispatcher resolves, writes the final result chunk and
     the chunked-encoding terminator.

- Cancellation interaction: if the dispatcher returns `Err(Cancelled)`
  the writer emits a JSON-RPC error frame with an application-defined
  `code` (`-32800`, used consistently across the codebase for
  "request cancelled") and `message: "request cancelled"`, then closes
  the stream cleanly. JSON-RPC reserves -32099..-32000 for application
  error codes; -32800 is outside the reserved range so it cannot
  collide with future spec additions.

The writer must be careful about partial writes: if a client closes the
TCP stream mid-flight, we propagate the IO error back into the
dispatcher path so the cancel token can be flipped (avoids zombie
helper loops streaming into the void).

### 4. Approval forwarding (polling design)

When `--mode2-approval forward` is set, the helper exposes two new tools
**alongside** `helper_ask`:

- `helper_pending_approvals({session_id}) -> [{request_id, tool_name, args, description, scope, requested_at}]`
- `helper_approve({session_id, request_id, decision: "approve"|"deny"})`

These tools are registered only when both `--mode2` and
`--mode2-approval forward` are set; under any other approval policy
they are absent from the registry so a hub agent with no need to drive
approvals never sees them. Mode 1's filtered registry never includes
them either — they have no meaning outside the helper's own
forwarding flow.

Internally, a process-wide `ApprovalQueue` holds per-session pending
requests:

```rust
pub struct ApprovalQueue {
    queues: Mutex<HashMap<String /* full session_id */, SessionQueue>>,
}
struct SessionQueue {
    pending: Vec<PendingApproval>,
    waiters: HashMap<String /* request_id */, oneshot::Sender<ApprovalDecision>>,
}
```

The forwarding `ApprovalHandler` lives at the `HelperAskTool` layer:

```rust
struct ForwardingApprovalHandler {
    queue: Arc<ApprovalQueue>,
    session_id: String,
    timeout: Duration,
    progress: Option<mpsc::Sender<StreamEvent>>, // for streaming hint
}

#[async_trait]
impl ApprovalHandler for ForwardingApprovalHandler {
    async fn request(&self, req: ApprovalRequest) -> ApprovalDecision {
        let (request_id, rx) = self.queue.enqueue(&self.session_id, req.clone());
        if let Some(sink) = &self.progress {
            let _ = sink.send(StreamEvent::ContentDelta(format!(
                "[awaiting approval] {} (request_id={})\n",
                req.tool_name, request_id
            ))).await;
        }
        match tokio::time::timeout(self.timeout, rx).await {
            Ok(Ok(decision)) => decision,
            Ok(Err(_)) => ApprovalDecision::Deny, // sender dropped
            Err(_) => {
                self.queue.timeout(&self.session_id, &request_id);
                ApprovalDecision::Deny
            }
        }
    }
}
```

`helper_pending_approvals` reads from the queue (without removing).
`helper_approve` looks up the waiter and sends through the oneshot.

**Caveats — call out in user-facing docs**:

1. **Hub LLM has to know to poll.** If the hub agent's prompt doesn't
   teach it to call `helper_pending_approvals` after kicking off a
   `helper_ask`, the helper will time out. We document this prominently
   in `docs/profile-as-mcp-mode2.md`.
2. **Streaming hint is best-effort.** When streaming is on, the helper
   emits a `[awaiting approval] tool=… request_id=…` progress event so a
   streaming hub agent has an explicit signal. Non-streaming hubs only
   see the queued state via polling.
3. **Timeout default 60s.** Configurable via
   `--mode2-approval-timeout-secs N`. Long enough for a hub LLM to
   notice and respond, short enough that a wedged hub doesn't pin a
   helper loop indefinitely.
4. **Per-session isolation.** The queue keys on the full `mode2:<fp>:<id>`
   session id from slice 1, so two unrelated hubs polling different
   sessions don't see each other's pending approvals.
5. **Garbage collection.** A queue entry is removed when answered or on
   timeout. Sessions whose queues drain to empty are removed too so a
   long-running server doesn't accumulate dead session keys.

### CLI surface delta (slice 2 additions)

```text
MODE 2
  --mode2                                Slice 1 (unchanged).
  --mode2-approval reject|allow_all|forward
                                         Adds 'forward'.
  --mode2-approval-timeout-secs N        forward only. Default 60.
  --mode2-max-iterations N               Slice 1 (unchanged).
  --mode2-call-timeout-secs N            Wall-clock per call. Default 0
                                         (unbounded).
  --mode2-disable-system-prompt          Reject helper_ask calls that
                                         pass system_prompt.
```

`requires = "mode2"` is enforced on every new flag the way slice 1
already enforces it on `--mode2-approval` and `--mode2-max-iterations`.

### Data flow (streaming + cancel + forward, all on)

```
hub                                                     helper
 │                                                        │
 │ POST /  { tools/call, _meta.progressToken: "p1" }      │
 ├───────────────────────────────────────────────────────▶│
 │                                                        │ register cancel(id) in CancelRegistry
 │                                                        │ build AgentLoop with ForwardingApprovalHandler
 │                                                        │ spawn AgentLoop::process_streamed_with_cancel
 │                                                        │
 │ SSE: event=message  notifications/progress (delta)     │
 │◀───────────────────────────────────────────────────────┤
 │                                                        │ ... agent runs ...
 │                                                        │ inner tool needs approval
 │                                                        │ enqueue PendingApproval, block on oneshot
 │ SSE: event=message  notifications/progress             │
 │       ("[awaiting approval] read_file id=ap-…")        │
 │◀───────────────────────────────────────────────────────┤
 │                                                        │
 │ POST /  helper_pending_approvals({session_id})         │
 ├───────────────────────────────────────────────────────▶│
 │ resp: [{request_id: ap-…, tool_name: read_file, …}]    │
 │◀───────────────────────────────────────────────────────┤
 │                                                        │
 │ POST /  helper_approve({session_id, ap-…, "approve"})  │
 ├───────────────────────────────────────────────────────▶│
 │                                                        │ oneshot fires, ApprovalDecision::Approve
 │ resp: {ok}                                             │
 │◀───────────────────────────────────────────────────────┤
 │                                                        │ ... agent continues, completes ...
 │ SSE: event=message  tools/call result {content, _meta} │
 │◀───────────────────────────────────────────────────────┤
 │                                                        │ unregister cancel(id)
```

Cancel flow (interrupt instead of approval):

```
hub                                                     helper
 │ POST /  notifications/cancelled {requestId: <id>}      │
 ├───────────────────────────────────────────────────────▶│ CancelRegistry.cancel(id)
 │ HTTP 202 Accepted                                      │ token cancelled
 │◀───────────────────────────────────────────────────────┤ AgentLoop drops out of select!
 │                                                        │ persist partial session
 │ SSE: tools/call error {code:-32800, message:"cancelled"}│
 │◀───────────────────────────────────────────────────────┤
```

## Testing strategy

### Unit tests

- `zunel-core/src/agent_loop.rs`:
  - `process_streamed_with_cancel_persists_partial_on_cancel`
  - `process_streamed_with_cancel_returns_cancelled_error`
  - `extra_system_message_prepends_before_skills_message`
- `zunel-cli/src/commands/mcp/cancel_registry.rs`:
  - `register_then_cancel_fires_token`
  - `cancel_unknown_id_is_noop`
  - `unregister_drops_token`
- `zunel-cli/src/commands/mcp/approval_queue.rs`:
  - `enqueue_returns_request_id_and_blocks_on_oneshot`
  - `approve_unblocks_waiter_with_decision`
  - `timeout_drops_waiter_and_returns_deny`
  - `pending_does_not_drain_queue`
- `zunel-cli/src/commands/mcp/helper_ask.rs`:
  - `helper_ask_passes_system_prompt_to_inner_loop`
  - `helper_ask_rejects_oversized_system_prompt`
  - `helper_ask_rejects_system_prompt_when_disabled`
  - `helper_ask_with_forward_policy_blocks_until_approved`
  - `helper_ask_with_forward_policy_times_out_to_deny`
- `zunel-cli/src/commands/mcp/forwarding_approval_test.rs`:
  - `forwarding_approval_emits_progress_hint_when_streaming`

### Integration tests (`zunel-mcp-self/tests/`)

- `streaming_dispatches_progress_then_result_when_token_present`
  — uses `MapDispatcher` to emit synthetic progress events; asserts SSE
  framing.
- `cancellation_notification_aborts_in_flight_call`
  — POSTs a slow streaming `tools/call`, then POSTs
  `notifications/cancelled`, asserts the SSE stream closes with the
  cancelled error frame.
- `forward_approval_two_clients_e2e`
  — two concurrent connections: one runs `helper_ask`, the other polls
  `helper_pending_approvals` and answers via `helper_approve`.

### Manual smoke

- `zunel mcp agent --mode2 --mode2-approval forward
  --mode2-approval-timeout-secs 30` from one terminal; an MCP Inspector
  on the URL with a streaming `tools/call`. Confirm progress frames and
  cancel work.

## Risks & mitigations

| Risk | Mitigation |
|---|---|
| **Hub agent doesn't poll for approvals.** Helper times out, call fails. | Document the polling pattern in `profile-as-mcp-mode2.md`. Streaming hint nudges streaming hubs. Operator can fall back to `--mode2-approval reject\|allow_all`. |
| **Slow client blocks the agent loop via channel back-pressure.** | Bounded channel (size 64) makes this self-limiting. The watchdog timeout (`--mode2-call-timeout-secs`) is a hard ceiling. |
| **Cancel races with completion.** Hub sends cancel just as the helper finishes. | The cancel registry's `cancel` method returns `bool`; if the entry was already removed, the notification is a no-op. The hub gets a normal final result frame either way. |
| **`system_prompt` injection.** A compromised hub could inject prompts that leak helper internals. | 8 KiB cap. Operator opt-out (`--mode2-disable-system-prompt`). The doc already flags "treat helper APIs as untrusted input"; this just adds a new vector with the same mitigation discipline. |
| **Session queue leak across crashed hubs.** | GC empty session queues; impose a hard cap (e.g., 16 pending per session) and fail enqueue with a structured error past it. |
| **Two `helper_approve` calls race for the same `request_id`.** | The oneshot is one-shot; the second send fails silently. The second `helper_approve` gets `is_error: true` ("approval already answered or expired"). |

## Out-of-scope follow-ups (slice 3+)

- Approval-forwarding via callback URL (operators who can run a hub HTTP
  server prefer this over polling).
- Bidirectional MCP GET / SSE channel — the spec-conformant way to do
  approval forwarding without polling.
- `system_prompt` allowlist (operators allowlist a finite set of
  per-call personas).
- Persistent token / cost budgeting.
- Hub-side helper for the polling pattern (a `mode2_drive` tool that
  wraps `helper_ask` + auto-poll).

## Decision log (slice-2 additions)

- **Streaming opt-in via `_meta.progressToken`** instead of a custom
  `--mode2-stream` flag. Rationale: the MCP spec already provides the
  signal; adding a CLI knob would just be a kill-switch we don't yet
  need.
- **Polling-based approval forwarding** instead of bidirectional SSE.
  Rationale: bidirectional SSE is a multi-week refactor of the HTTP
  transport on its own; polling tools fit the existing one-way request
  model and ship in days. We accept the documented "hub LLM must know
  to poll" UX cost.
- **`process_streamed_with_cancel` as an additive method** rather than
  changing `process_streamed`'s signature. Rationale: every existing
  caller (CLI, gateway, tests) keeps working unchanged; the new method
  is the strictly-richer entry point.
- **Operator persona stacks above skills message.** Rationale: skills
  describe capabilities, persona describes role; standard system-message
  layering makes persona top-most.
