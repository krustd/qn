# qn

Run a command and send its completion result to an HTTP endpoint.

`qn` reports the command, exit code, elapsed time, working directory, and—when requested—its standard output and standard error. It sends a notification for every wrapped command, including commands that finish immediately; it does not apply a duration threshold. Notification failures are written to standard error and do not change the command's exit status.

## Installation

```bash
cargo install krust-qn
```

## Quick start

Configure an endpoint, token, and device name in an interactive terminal:

```bash
qn init
```

Then run a command through `qn`, or send text, Markdown, images, and files directly:

```bash
qn sleep 30
qn make -j12
qn --shell "git push && cargo build --release"
qn -t "预览环境部署完成"
qn -m "# 预览环境部署完成"
qn -i ./preview.png
qn -f ./report.pdf
```

Without configuration, `qn` still runs commands but prints a reminder and skips their notifications. Direct actions require configuration and fail if they cannot be sent.

## Usage

```
qn [-a|--attach-output] [--no-notify] <command> [args...]
qn [-a|--attach-output] [--no-notify] --shell <command-string>
qn -t|--text <content>
qn -m|--markdown <content>
qn -i|--image <path>
qn -f|--file <path>
qn --status
qn init
qn init-shell <fish|bash|zsh>
```

| Option | Behavior |
|--------|----------|
| `-a`, `--attach-output` | Capture standard output and standard error, replay them after the command exits, and add them to the notification. Output is therefore not streamed live while the command runs. |
| `--no-notify` | Run the command without checking configuration or sending a notification. |
| `-t <content>`, `--text <content>` | Send a plain-text message without running a command. The device name is prefixed to the content. |
| `-m <content>`, `--markdown <content>` | Send Markdown without rewriting its content. |
| `-i <path>`, `--image <path>` | Upload and send the path as an image message. |
| `-f <path>`, `--file <path>` | Upload and send the path as a downloadable file attachment. |
| `--status` | Print QQ Gateway and default-recipient binding status. |
| `--` | Stop parsing `qn` options; the remaining arguments are the command to run. |

`-t`, `-m`, `-i`, `-f`, and `--status` are direct actions. They cannot be combined with command-execution options. Image and file mode are explicit: `qn -f image.png` sends a downloadable attachment, while `qn -i unknown.bin` requests an image message.

Before the command name, every argument beginning with `-` belongs to `qn`. Unknown `qn` options are rejected with status `2` and never run the following command. After the command name, arguments are passed through unchanged:

```bash
qn --no-notify cargo build --release
qn -- -program-with-leading-hyphen argument
```

For ordinary command arguments, the executable starts the requested program directly. `qn` exits with that command's exit code; a failed notification does not replace it.

## Configuration

`qn init` requires an interactive terminal and writes this file:

```text
~/.config/qn/config
```

It obtains values in this order:

1. `QN_ENDPOINT` — the QQ Task Notifier server root URL. If unset, `qn init` prompts for it; press Enter to use `https://krust.iepose.cn`.
2. `QN_TOKEN` — the required Bearer token, always prompted by `qn init`.
3. Device name — defaults to the machine hostname. On macOS, if the hostname is `localhost`, qn uses the configured LocalHostName instead.

The resulting file contains `endpoint=...`, `token=...`, and `name=...`. On Unix, files created by qn are written with permission `0600`. Running `qn init` replaces the file with the newly entered values. Existing configurations ending in `/task-completed` must run `qn init` once to switch to the server root URL.

At notification time, these environment variables override the corresponding file values:

| Environment variable | Config key | Purpose |
|----------------------|------------|---------|
| `QN_ENDPOINT` | `endpoint` | QQ Task Notifier server root URL |
| `QN_TOKEN` | `token` | Bearer token |

`name` has no environment-variable override. If it is missing from an existing config file, `qn` uses the hostname and appends it to the file.

## Shell integration

Shell integration is optional. It makes `qn <command>` a shell function, so shell builtins and shell-specific command strings can be run in the current shell before `qn` sends the completion report.

Install it explicitly for one of the supported shells:

```bash
qn init-shell fish
qn init-shell bash
qn init-shell zsh
```

`init-shell` does not auto-detect the shell. It writes a qn-managed function block to the selected startup file; rerunning it updates that block in place instead of appending a duplicate. Restart the selected shell after installation:

| Shell | Startup file |
|-------|--------------|
| Fish | `~/.config/fish/config.fish` |
| Bash | `~/.bashrc` |
| Zsh | `~/.zshrc` |

After upgrading, run `qn init-shell <shell>` once for the current shell to update its function block, including direct-action and option validation support.

## Notification requests

qn sends wrapped commands and `qn -t` to `POST /v1/messages` under `QN_ENDPOINT`:

```http
Authorization: Bearer <token>
Content-Type: application/json

{"summary":"device-name\n任务完成\n命令：make -j12\n退出码：0\n耗时：2m 15s\n工作目录：/home/me/project"}
```

A nonzero command exit code changes `任务完成` to `任务失败`. With `--attach-output`, the summary also includes an `输出：` section containing the captured standard output and/or standard error.

All native routes use the same server root:

| Action | Request |
|--------|---------|
| Wrapped command, `qn -t ...` | `POST /v1/messages` |
| `qn -m ...` | `POST /v1/markdown` with verbatim `content` |
| `qn -i ...` | `POST /v1/media` multipart form with `file_type=image` |
| `qn -f ...` | `POST /v1/media` multipart form with `file_type=file` |
| `qn --status` | `GET /status` |

[QQ Task Notifier](https://github.com/krustd/qq-task-notifier) binds exactly one default recipient; qn does not allow callers to override it.

## How it works

1. For a command invocation, `qn` runs the requested command and records its exit code and elapsed time.
2. Direct actions submit their requested text, Markdown, image, file, or status operation immediately.
3. Wrapped commands and plain text add the configured device name; Markdown content is sent unchanged.
4. A command-notification transport error or non-success HTTP status is reported as a warning; the wrapped command's exit code is retained. Direct-action delivery errors exit with status `1`.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
