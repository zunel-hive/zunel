# zunel

Zunel is a lean personal AI assistant with one supported stack:

- local CLI chat via `zunel agent`
- Slack-backed gateway via `zunel gateway`
- two provider paths:
  - `providers.custom` — any OpenAI-compatible endpoint (API key + base URL)
  - `providers.codex` — ChatGPT Codex via your local `codex` CLI OAuth login

Config, runtime state, logs, cron data, and the default workspace live under
`~/.zunel`.

## Install

The safest path is a source checkout:

```bash
pip install -e .
```

If you want a fixed build, check out a specific tag or commit and install that
revision with `pip install .`.

## Quick Start

1. Create the default config and workspace:

   ```bash
   zunel onboard
   ```

   This creates `~/.zunel/config.json` and `~/.zunel/workspace/`.

2. Edit `~/.zunel/config.json` and configure `providers.custom`:

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

   Point `apiBase` at any OpenAI-compatible endpoint you want to use.

   Alternatively, if you are already signed in with the `codex` CLI, you can
   use the ChatGPT Codex Responses API — no API key needed:

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

3. Start a local chat session:

   ```bash
   zunel agent
   ```

   For a one-shot prompt:

   ```bash
   zunel agent -m "Hello, who are you?"
   ```

## Slack Gateway

Add Slack credentials to `~/.zunel/config.json`:

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

Then start the gateway:

```bash
zunel gateway
```

The gateway runs Slack, cron, Dream, and heartbeat using the same workspace and
agent defaults as the CLI.

## Commands

```text
zunel onboard            # initialize or refresh config + workspace
zunel agent              # interactive CLI chat
zunel gateway            # start the Slack-backed gateway
zunel status             # show provider, model, and workspace status
zunel channels status    # show Slack channel status
```

## Programmatic Use

```python
import asyncio
from zunel import Zunel


async def main() -> None:
    bot = Zunel.from_config("~/.zunel/config.json")
    result = await bot.run("Summarize this repo.")
    print(result.content)


asyncio.run(main())
```

## Scope Of This Build

This repo intentionally documents and supports only the lean Zunel surface:

- the local CLI agent
- the Slack gateway
- the `providers.custom` OpenAI-compatible provider path
- the `providers.codex` ChatGPT Codex OAuth path (requires a working `codex`
  CLI login)
- Python usage through `from zunel import Zunel`
- config, workspace, and runtime state under `~/.zunel`

The following upstream surfaces are intentionally out of scope here:

- extra chat integrations such as Telegram, WhatsApp, Discord, Feishu, Matrix,
  QQ, WeCom, WeiXin, and MS Teams
- vendor-specific login or OAuth flows beyond the Codex path listed above
  (GitHub Copilot, Anthropic-specific, Azure OpenAI-specific paths, etc.)
- standalone HTTP serving, API server flows, or `serve`
- browser, bridge, websocket, and WebUI surfaces
