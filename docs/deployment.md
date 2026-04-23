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
docker run -v ~/.zunel:/home/zunel/.zunel -p 18790:18790 zunel gateway

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

## Health Endpoint

`zunel gateway` exposes a lightweight health endpoint on `gateway.host` and
`gateway.port`:

- `GET /health` returns `{"status":"ok"}`
- other paths return `404`

By default the gateway binds to `127.0.0.1:18790`, so the endpoint stays local.

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
