# qn

Shell-aware command completion notifications.

`qn` wraps shell commands and sends a completion notification — exit code, duration, working directory, and optionally stdout/stderr — to a configurable HTTP endpoint. Pair it with [QQ Task Notifier](https://github.com/krustd/qqbot) to get push notifications in QQ when long-running tasks finish.

## Installation

```bash
cargo install krust-qn
```

Prebuilt binaries are not yet provided; `cargo install` builds from source.

## Quick start

```bash
# One-time setup — configure endpoint, token, and device name
qn init

# Wrap any command
qn sleep 30
qn make -j12
qn --shell "git push && cargo build --release"
```

After the command finishes, `qn` sends a notification like:

```
my-server
任务完成
命令：make -j12
退出码：0
耗时：2m 15s
工作目录：/home/me/project
```

## Shell integration

Shell integration makes `qn` a transparent wrapper: wrap commands with `qn` instead of remembering flags, and get notifications automatically. Long-running commands (≥30s) are detected; short ones skip the notification.

```bash
# Install for your shell (auto-detected)
qn init-shell bash
qn init-shell zsh
qn init-shell fish
```

This appends a shell function to your rc file. After restarting your shell or sourcing the rc file:

```bash
# Just prefix any command with qn
qn tar -czf backup.tar.gz /data
qn docker build -t myapp .
```

To print the shell function without modifying rc files (e.g. for plugin managers):

```bash
qn shell-init fish >> ~/.config/fish/conf.d/qn.fish
```

## Usage

```
qn [-a|--attach-output] [--no-notify] <command> [args...]
qn [-a|--attach-output] [--no-notify] --shell <command-string>
qn init
qn shell-init <fish|bash|zsh>
qn init-shell <fish|bash|zsh>
```

| Flag | Effect |
|------|--------|
| `-a`, `--attach-output` | Include stdout and stderr in the notification |
| `--no-notify` | Run the command but skip the notification |

## Configuration

`qn init` walks through setup interactively:

1. **Device name** — identifies this machine in notifications (defaults to hostname).
2. **Endpoint URL** — where to POST the notification. Default: `https://krust.iepose.cn/task-completed`, which is the [QQ Task Notifier](https://github.com/krustd/qqbot) API.
3. **Token** — Bearer token for the endpoint.

All values are stored in `~/.config/qn/config` and can also be set via environment variables:

| Env var | Config key | Description |
|---------|-----------|-------------|
| `QN_ENDPOINT` | `endpoint` | Notification API URL |
| `QN_TOKEN` | `token` | Bearer token |

Environment variables take precedence over the config file.

## Server-side: QQ Task Notifier

`qn` sends notifications to any HTTP endpoint that accepts:

```
POST /task-completed
Authorization: Bearer <token>
Content-Type: application/json

{"summary": "device-name\n..."}
```

The companion [QQ Task Notifier](https://github.com/krustd/qqbot) provides this endpoint and forwards messages to QQ private chat. Deploy it alongside `qn` for push notifications on your phone.

Quick server setup:

```bash
git clone https://github.com/krustd/qqbot.git
cd qqbot
# Edit docker-compose.yml with your QQ Bot credentials
docker compose up -d
```

Then configure `qn` to point at the server:

```bash
export QN_ENDPOINT="http://your-server:8765/task-completed"
export QN_TOKEN="your-api-token"
qn init   # or set these values interactively
```

## How it works

1. `qn` spawns the requested command as a child process.
2. When the command exits, it captures the exit code, elapsed time, and optionally stdout/stderr.
3. It POSTs a JSON summary to the configured endpoint.
4. The server (e.g. QQ Task Notifier) delivers the notification to your messaging platform.

Shell integration uses a shell function that intercepts `qn <command>` calls: the function runs the command, then calls `qn __notify` with the exit code and timing to send the notification without a second HTTP round-trip for command execution.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
