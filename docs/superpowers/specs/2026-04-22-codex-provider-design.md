# Codex Provider Support Design

## Summary

Add a first-class `codex` provider to the lean `zunel` build so users can drive
their local ChatGPT Codex Responses endpoint through `zunel` without routing
through the generic `custom` / OpenAI-compatible path.

The provider is a direct HTTP client against the Codex Responses API, reusing
the existing OpenAI Responses converter/SSE helpers, and it authenticates using
the user's already-installed Codex local OAuth state (the same state the
`codex` CLI uses). Users configure it explicitly through
`agents.defaults.provider = "codex"` (e.g. `model = "gpt-5.4"`).

## Goals

- Let users run `zunel` backed by ChatGPT Codex using their existing local
  `codex` login, with no re-authentication inside `zunel`.
- Keep the selection explicit: `codex` is a distinct provider spec, not an
  implicit fallback behind `custom`.
- Reuse existing Responses-API infrastructure (`convert_messages`,
  `convert_tools`, SSE consumption) so tool-calling, reasoning, and streaming
  behave consistently with other OpenAI-Responses-style providers.
- Surface the provider in onboarding and docs so it is discoverable.

## Non-Goals

- Shelling out to the `codex` CLI at runtime (not using `codex exec` or a
  subprocess boundary).
- Implementing a new OAuth flow inside `zunel`. This design only reads the
  existing local Codex auth state.
- Backwards-compatibility with the old `nanobot` / `openai-codex/*` model
  prefix. Under `zunel`, the provider is `codex` and the model is a plain
  string such as `gpt-5.4`.
- Restoring any other previously-removed providers.
- Automated end-to-end tests that hit the live Codex endpoint. Live validation
  is a manual smoke step, not CI.

## Current Context

The lean `zunel` build currently ships one real provider family
(`OpenAICompatProvider`, selectable as `custom` / `openai` / `openai_compat`).
The broader repo history includes a previous `OpenAICodexProvider` that:

- Called `https://chatgpt.com/backend-api/codex/responses`.
- Read an OAuth token via `oauth_cli_kit.get_token()`.
- Reused the OpenAI Responses converter + SSE parser.
- Was wired into the provider registry and onboarding as `openai_codex`.

That implementation was deleted during the lean cleanup. This design
reintroduces an equivalent provider under the renamed brand and the lean
provider surface.

The user's local machine already has a working `codex` CLI install. The
running agent process does not currently have read access to
`~/.codex/config.toml`, so any design that reads local Codex state must do so
in a way that works under the user's normal shell permissions (i.e. when they
run `zunel` themselves) without requiring the agent itself to read that file.

## Product Shape After This Change

After this change, configuration exposes a new provider entry:

```json
{
  "providers": {
    "codex": {}
  },
  "agents": {
    "defaults": {
      "provider": "codex",
      "model": "gpt-5.4"
    }
  }
}
```

- `providers.codex` does not require an `apiKey`.
- `providers.codex.apiBase` is accepted as an optional override, primarily for
  debugging against a staging endpoint. It **only** replaces the request URL;
  authentication (OAuth token, account id header) is unchanged. If unset,
  the default Codex Responses URL is used.
- `agents.defaults.model` for this provider is a plain model identifier such
  as `gpt-5.4` (the user-requested default) — no `openai-codex/` prefix. The
  provider does not validate model availability; the Codex backend returns a
  clear error if the selected model is not accessible to the signed-in
  account, and that error surfaces to the user unchanged.
- `zunel onboard` lists `codex` alongside `custom` as a selectable provider.
- Docs describe `codex` as "use your local ChatGPT Codex login" with a short
  pointer at the `codex` CLI for establishing that login.

## Code Structure Design

### New provider module

Add `zunel/providers/codex_provider.py` containing `CodexProvider`, a
`LLMProvider` subclass that:

- Does not take `api_key` / `api_base` in its constructor positionally; config
  supplies an optional base URL override.
- Reuses `zunel.providers.openai_responses.{convert_messages, convert_tools,
  consume_sse}` for request/response translation.
- Obtains OAuth credentials via the same local OAuth helper used by the
  previous implementation (`oauth_cli_kit.get_token`), invoked off the event
  loop (`asyncio.to_thread`) to avoid blocking.
- Builds the Codex-specific request headers (`Authorization`,
  `chatgpt-account-id`, `OpenAI-Beta: responses=experimental`, `originator`,
  `User-Agent`, `accept: text/event-stream`, `content-type: application/json`).
  The `originator` value is intentionally **not** renamed to `zunel` up front:
  the Codex backend may whitelist specific originator strings, so the initial
  implementation keeps a known-accepted value (e.g. `codex_cli_rs`, matching
  the upstream Codex CLI) and documents this in a code comment. The
  `User-Agent` is a plain `zunel` string and can be rebranded freely. If
  Codex rejects a given originator in practice, this value is the one to
  adjust first.
- Issues a streaming POST against the Codex Responses URL and returns an
  `LLMResponse` with `content`, `tool_calls`, and `finish_reason`.
- Supports the same public interface as other providers: `chat`,
  `chat_stream`, `get_default_model`.
- Default model is `gpt-5.4` (the model the user explicitly wants to test),
  with no `openai-codex/` prefix.

Implementation notes:

- Reuse the previous implementation's SSL retry fallback only if required in
  practice; otherwise keep it strict (`verify=True`). Any fallback must log a
  warning with enough detail to be actionable.
- Use `loguru` for logging, consistent with other providers in the codebase.
- Prompt-cache key computation follows the same hash-of-messages pattern as
  the previous implementation, so request caching on the Codex side remains
  stable.

### Provider registry and selection

Register the new provider in `zunel/providers/registry.py`:

- Add a `ProviderSpec` with name `codex`.
- `requires_api_key = False` (authentication is via local OAuth).
- Optional `default_api_base` equal to the Codex Responses URL so overrides
  can replace it.
- Factory instantiates `CodexProvider(default_model=model)`.

Update `zunel/providers/__init__.py` so `CodexProvider` is importable
alongside the existing providers, following the same lazy-import pattern.

### Config schema

In `zunel/config/schema.py`:

- Add a `CodexProviderConfig` (or equivalent) with optional `api_base` and no
  required `api_key`.
- Include it in the provider map so `providers.codex` validates without an
  `apiKey`.
- Ensure `AgentDefaults.provider = "codex"` is accepted.

### Onboarding

In `zunel/cli/onboard.py` (or the helper currently used to enumerate providers
for onboarding, e.g. `_get_provider_names()`):

- Add `codex` to the list of selectable providers.
- When the user picks `codex`, the wizard skips the API-key prompt and
  instead prints a short note: "Uses your local ChatGPT Codex login. Make
  sure you are signed in with the `codex` CLI."
- Default model suggestion for `codex` is `gpt-5.4`.

### Documentation

- `README.md`: mention `codex` as a supported provider for users who already
  use ChatGPT Codex locally, with a one-line prerequisite note.
- `docs/configuration.md`: document the `providers.codex` block, including the
  optional `apiBase` override and the authentication prerequisite.
- `docs/quick-start.md` / `docs/cli-reference.md`: a short "using Codex"
  subsection referring to onboarding.

## Data Flow and Behavior Expectations

1. User runs `zunel onboard` and selects `codex` as the provider.
2. Wizard writes `providers.codex: {}` and
   `agents.defaults.provider = "codex"` with a selected `model` (e.g.
   `gpt-5.4`).
3. On first chat turn:
   - `CodexProvider` calls `get_codex_token()` off the event loop.
   - Builds headers and body per the Responses API contract.
   - Streams deltas via `consume_sse`.
   - Emits normal `LLMResponse` content, tool calls, and finish reason.
4. Tool-calling, reasoning-effort, and parallel tool calls behave the same as
   other OpenAI-Responses-style providers in this codebase.

No behavioral differences versus `custom` beyond authentication and endpoint
selection are intended.

## Error Handling Expectations

- If `oauth_cli_kit.get_token()` raises (no login, corrupted state, expired
  token that cannot refresh), `CodexProvider` must surface a clear error
  string in the returned `LLMResponse` (finish_reason `error`) that tells the
  user to run the `codex` CLI login.
- HTTP 401/403 from the Codex endpoint should produce a user-visible error
  that explains the token likely needs refresh.
- HTTP 429 should map to a retry-after-aware error response consistent with
  the existing provider retry plumbing.
- Any generic HTTP failure should be surfaced with status code and a brief
  body excerpt, again consistent with existing providers.
- There is no silent fallback to `custom` or the OpenAI-compatible endpoint.
  Selecting `codex` means the user explicitly opted into this path.

## Testing Strategy

### Unit tests (new)

Add `tests/providers/test_codex_provider.py` covering:

- Header construction: `Authorization` uses the OAuth token, account id is
  propagated via `chatgpt-account-id`, `User-Agent` is a `zunel` string, and
  `originator` is the known-accepted value documented in the implementation
  (see the originator note above).
- Request body: `model` is passed through without the old `openai-codex/`
  prefix; `stream: True`; `store: False`; `tool_choice` default is `auto`;
  `reasoning.effort` is forwarded when provided; `tools` are converted via
  `convert_tools`.
- Streaming: content deltas from a mocked SSE stream surface through the
  `on_content_delta` callback.
- Tool-call parsing: Codex SSE events that represent tool calls produce the
  expected `ToolCallRequest` objects.
- Error handling: non-200 response surfaces a clear error; missing/expired
  OAuth token produces a user-friendly error.

### Registry / init tests (updated)

- `tests/providers/test_providers_init.py`: assert `CodexProvider` is
  importable lazily and the registry exposes a `codex` spec with
  `requires_api_key = False`.

### Onboarding test (updated)

- `tests/agent/test_onboard_logic.py`: update `_get_provider_names()`-style
  assertions to include `codex` and verify the wizard does not prompt for an
  API key in the `codex` branch.

### Config regression test

- Add a test under `tests/config/` asserting that a config with
  `providers.codex: {}` and `agents.defaults.provider = "codex"` validates
  without raising and without requiring `apiKey`.

### Lean-surface sanity

- `tests/test_lean_surface.py`: no new assertions required, but ensure
  existing "supported providers" checks (if any) include `codex` rather than
  treating it as an alias of `custom`.

### Manual end-to-end validation (outside CI)

- On a machine with a valid local Codex login, run
  `ZUNEL_CONFIG_DIR=... zunel agent` with `provider = "codex"` and
  `model = "gpt-5.4"`, send a short prompt, and confirm a streamed response
  plus a successful tool call. This step is documented in the PR description
  or follow-up notes; it is not automated.

### Lint and hygiene

Run `ruff` on all touched files and fix any diagnostics introduced by the
change.

## Risks and Mitigations

### Risk: agent process cannot read `~/.codex/`

The assistant process currently cannot read the user's local Codex config due
to filesystem permissions. If `CodexProvider` ever needs to read that path
directly, any test harness or dev environment that lacks those permissions
will fail in confusing ways.

Mitigation: rely only on the already-established `oauth_cli_kit.get_token`
path (which the user's own `zunel` process runs with their own permissions).
Do not read `~/.codex/config.toml` from Python. If future work needs config
from that file, it must be introduced with explicit docs and a fallback path.

### Risk: Codex API contract drift

The Codex Responses endpoint is not a stable public API. Headers, body
fields, or SSE event shapes may change.

Mitigation: concentrate Codex-specific request shape in one module
(`codex_provider.py`) and keep response parsing inside the shared
`openai_responses` helpers, which already track Responses-style drift for the
other providers. Regression tests use recorded SSE fixtures and should be
updated alongside any upstream change.

### Risk: silent reuse of `custom` path

If a user already has `providers.custom` configured and picks `codex` in
onboarding, the wizard or runtime could accidentally fall back to the
`custom` provider when the OAuth token fails.

Mitigation: strict routing. Selecting `codex` always instantiates
`CodexProvider`. Any failure surfaces as a user-visible error, never as a
silent reroute. Tests assert that picking `codex` does not route through
`OpenAICompatProvider`.

### Risk: re-introducing removed dependencies

`oauth-cli-kit` was removed from `pyproject.toml` during the lean cleanup.

Mitigation: explicitly re-add `oauth-cli-kit>=0.1.3,<1.0.0` under
`[project].dependencies` as part of this change. Document the addition in the
PR description so reviewers see the dependency delta.

## Recommended Execution Order

1. Add `oauth-cli-kit` back to `pyproject.toml` dependencies.
2. Port the previous `OpenAICodexProvider` into
   `zunel/providers/codex_provider.py`, renaming it `CodexProvider` and
   updating branding/strings.
3. Register `codex` in `zunel/providers/registry.py` and expose it in
   `zunel/providers/__init__.py`.
4. Extend `zunel/config/schema.py` so `providers.codex` validates without an
   API key.
5. Update onboarding (`zunel/cli/onboard.py`) so `codex` is selectable and
   does not prompt for an API key.
6. Update `README.md`, `docs/configuration.md`, `docs/quick-start.md`, and
   `docs/cli-reference.md` to document the new provider.
7. Add/update tests:
   - `tests/providers/test_codex_provider.py` (new)
   - `tests/providers/test_providers_init.py` (updated)
   - `tests/agent/test_onboard_logic.py` (updated)
   - `tests/config/` regression test (new or extended)
8. Run `ruff` + targeted `pytest` for touched surfaces.
9. Manual smoke: run `zunel agent` against `gpt-5.4` on a machine with a live
   Codex login and confirm streaming + tool use.

## Success Criteria

This design is successful when all of the following are true:

- `providers.codex` is a first-class selectable provider in config,
  onboarding, and docs, not an alias of `custom`.
- `CodexProvider` uses the local Codex OAuth state to call the Codex
  Responses endpoint and streams responses via the existing OpenAI Responses
  helpers.
- `agents.defaults.provider = "codex"` with `model = "gpt-5.4"` passes config
  validation without requiring an `apiKey`.
- Unit tests cover request construction, streaming, tool calls, and error
  paths, and they pass in CI.
- Onboarding surfaces `codex` and skips the API-key prompt for it.
- Docs describe the prerequisite of being signed in through the `codex` CLI
  and do not imply a fallback to `custom`.
- A manual end-to-end run against `gpt-5.4` succeeds on the author's Mac.
- No removed-channel/provider surfaces are re-introduced by this change, and
  `ruff`/lean-surface regressions remain clean.
