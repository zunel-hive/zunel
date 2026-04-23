# Deployment

The lean Zunel build has two deployment modes:

- run `zunel agent` locally when you want an interactive CLI
- run `zunel gateway` as a long-lived Slack gateway

Containers and services run the same CLI and gateway entrypoints documented
here. There is no second runtime to deploy beyond the local CLI and the
Slack-backed gateway.

## Docker Compose

The bundled `docker-compose.yml` defines two services:

- `zunel-gateway` for the long-running Slack gateway
- `zunel-cli` for one-off interactive or maintenance commands

First-time setup:

```bash
docker compose run --rm zunel-cli onboard
$EDITOR ~/.zunel/config.json
docker compose up -d zunel-gateway
```

Common operations:

```bash
docker compose run --rm zunel-cli agent -m "Hello!"
docker compose run --rm zunel-cli status
docker compose logs -f zunel-gateway
docker compose down
```

## Direct Docker

```bash
# Build the image
docker build -t zunel .

# Initialize config (first time only)
docker run -v ~/.zunel:/home/zunel/.zunel --rm zunel onboard

# Edit config on the host
$EDITOR ~/.zunel/config.json

# Start the Slack gateway
docker run -v ~/.zunel:/home/zunel/.zunel zunel gateway

# Run one-off CLI commands
docker run -v ~/.zunel:/home/zunel/.zunel --rm zunel agent -m "Hello!"
docker run -v ~/.zunel:/home/zunel/.zunel --rm zunel status
```

> [!TIP]
> Mount `~/.zunel` into the container so config, workspace, cron data, and logs
> persist across restarts.
>
> The image runs as user `zunel` (UID 1000). If you hit a permission error on
> the mounted directory, either fix ownership on the host with
> `sudo chown -R 1000:1000 ~/.zunel` or run the container as your own UID with
> `--user $(id -u):$(id -g)`. Podman users can use `--userns=keep-id`.

## Communication Surfaces

`zunel gateway` does not bind any network port. The only ways to talk to a
running gateway are:

- the **Slack channel** configured under `channels.slack` (if enabled)
- the **`zunel` CLI** on the same host against the same `--config` /
  `--workspace`

There is no HTTP server, health endpoint, or webhook listener. If you need a
liveness signal, use the process supervisor (systemd, Docker, launchd) — if
`zunel gateway` is running, the services are up.

## Linux Service

For a persistent Slack gateway on Linux, run `zunel gateway` as a systemd user
service.

### 1. Find the binary path

```bash
which zunel
```

### 2. Create `~/.config/systemd/user/zunel-gateway.service`

```ini
[Unit]
Description=Zunel Gateway
After=network.target

[Service]
Type=simple
ExecStart=%h/.local/bin/zunel gateway
Restart=always
RestartSec=10
NoNewPrivileges=yes
ProtectSystem=strict
ReadWritePaths=%h

[Install]
WantedBy=default.target
```

### 3. Enable and start it

```bash
systemctl --user daemon-reload
systemctl --user enable --now zunel-gateway
```

Common operations:

```bash
systemctl --user status zunel-gateway
systemctl --user restart zunel-gateway
journalctl --user -u zunel-gateway -f
```

If you edit the service file itself, run `systemctl --user daemon-reload`
before restarting.

To keep the user service running after logout:

```bash
loginctl enable-linger $USER
```
