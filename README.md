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

Then run a command through `qn`:

```bash
qn sleep 30
qn make -j12
qn --shell "git push && cargo build --release"
```

Without configuration, `qn` still runs the command but prints a reminder and skips the notification.

## Usage

```
qn [-a|--attach-output] [--no-notify] <command> [args...]
qn [-a|--attach-output] [--no-notify] --shell <command-string>
qn init
qn init-shell <fish|bash|zsh>
```

| Option | Behavior |
|--------|----------|
| `-a`, `--attach-output` | Capture standard output and standard error, replay them after the command exits, and add them to the notification. Output is therefore not streamed live while the command runs. |
| `--no-notify` | Run the command without checking configuration or sending a notification. |
| `--shell <command-string>` | Run the string with `sh -c` on non-Windows systems (`cmd /C` on Windows) when using the executable directly. With shell integration loaded, the string is evaluated by the current shell instead. |
| `--` | Stop parsing `qn` options; the remaining arguments are the command to run. |

For ordinary command arguments, the executable starts the requested program directly. `qn` exits with that command's exit code; a failed notification does not replace it.

## Configuration

`qn init` requires an interactive terminal and writes this file:

```text
~/.config/qn/config
```

It obtains values in this order:

1. `QN_ENDPOINT` — the endpoint URL. If this environment variable is unset, `qn init` prompts for it; press Enter to use the default, `https://krust.iepose.cn/task-completed`.
2. `QN_TOKEN` — the required Bearer token, always prompted by `qn init`.
3. Device name — defaults to the machine hostname.

The resulting file contains `endpoint=...`, `token=...`, and `name=...`. On Unix, files created by `qn` are written with permission `0600`. Running `qn init` replaces the file with the newly entered values.

At notification time, only these environment variables override the corresponding file values:

| Environment variable | Config key | Purpose |
|----------------------|------------|---------|
| `QN_ENDPOINT` | `endpoint` | HTTP endpoint URL |
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

`init-shell` 不会自动识别 Shell。它会把由 `qn` 管理的函数块写入对应启动文件；重复执行会原位更新，不会重复追加。执行后重启对应 Shell 生效：

| Shell | Startup file |
|-------|--------------|
| Fish | `~/.config/fish/config.fish` |
| Bash | `~/.bashrc` |
| Zsh | `~/.zshrc` |

从旧版本升级后，针对当前 Shell 再执行一次 `qn init-shell <shell>`，即可将旧的加载配置迁移为函数块。

## Notification request

`qn` sends one HTTP `POST` request to the configured endpoint after the command completes. The endpoint path is part of `QN_ENDPOINT`; `/task-completed` is only the path in the default URL.

```http
Authorization: Bearer <token>
Content-Type: application/json

{"summary":"device-name\n任务完成\n命令：make -j12\n退出码：0\n耗时：2m 15s\n工作目录：/home/me/project"}
```

The JSON body always has one `summary` string. A nonzero command exit code changes `任务完成` to `任务失败`. With `--attach-output`, the summary also includes an `输出：` section containing the captured standard output and/or standard error.

Any HTTP service that accepts this request format can be used. [QQ Task Notifier](https://github.com/krustd/qq-task-notifier) is an optional companion service for forwarding these notifications to QQ.

## How it works

1. `qn` runs the requested command and records its exit code and elapsed time.
2. It builds a Chinese-language summary with the current working directory and configured device name.
3. If notifications are enabled and both endpoint and token are available, it POSTs `{"summary": ...}` with Bearer authentication.
4. A transport error or non-success HTTP status is reported as a warning; the wrapped command's exit code is retained.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
