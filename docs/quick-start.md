# Install and Quick Start

## Install

Zunel is currently best installed from a source checkout.

**Editable install** (best for development and local changes):

```bash
pip install -e .
```

**Pinned install** (best when you want a conservative, fixed checkout):

```bash
git checkout <tag-or-commit>
pip install .
```

If you prefer `uv`, run the same commands from the checkout with `uv pip install -e .`
or `uv pip install .`.

This lean build does not require extra browser-side or multi-channel setup
steps.

## Quick Start

### 1. Initialize

```bash
zunel onboard
```

This creates:

- `~/.zunel/config.json`
- `~/.zunel/workspace/`

Use `zunel onboard --wizard` if you want the interactive onboarding flow.

### 2. Configure a provider

Zunel ships two provider paths. Pick one by editing `~/.zunel/config.json`.

#### Option A — OpenAI-compatible endpoint (`providers.custom`)

```json
{
  "providers": {
    "custom": {
      "apiKey": "sk-...",
      "apiBase": "https://api.openai.com/v1"
    }
  },
  "agents": {
    "defaults": {
      "provider": "custom",
      "model": "gpt-4o-mini"
    }
  }
}
```

`apiBase` can point at any OpenAI-compatible service you trust. `apiKey` is
required by the current runtime, even if your endpoint uses a placeholder value.

#### Option B — ChatGPT Codex via local OAuth (`providers.codex`)

First, sign in with the `codex` CLI (once, using your ChatGPT account). Then:

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

No API key is needed — `zunel` reads the local Codex OAuth token. If you
haven't signed in yet, `zunel` will return a clear error pointing you to
`codex` CLI login.

### 3. Start the local agent

```bash
zunel agent
```

For a one-shot prompt:

```bash
zunel agent -m "Summarize this repo."
```

### 4. Optional: enable Slack and start the gateway

Add a Slack block to `~/.zunel/config.json`:

```json
{
  "channels": {
    "slack": {
      "enabled": true,
      "mode": "socket",
      "botToken": "xoxb-...",
      "appToken": "xapp-...",
      "allowFrom": ["*"]
    }
  }
}
```

Then run:

```bash
zunel gateway
```

This is the only built-in gateway/channel path documented in the lean build.

### 5. Sanity-check the setup

```bash
zunel status
zunel channels status
```

At this point you are ready to use:

- `zunel agent` for local work
- `zunel gateway` for Slack-backed automation and chat
