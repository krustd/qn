use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::Instant;

use chardetng::EncodingDetector;
use directories::BaseDirs;
use encoding_rs::{Encoding, UTF_16BE, UTF_16LE};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};

const DEFAULT_ENDPOINT: &str = "https://krust.iepose.cn";
const SHELL_INTEGRATION_ENV: &str = "QN_SHELL_INTEGRATION";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShellKind {
    Fish,
    Bash,
    Zsh,
    PowerShell,
}

impl ShellKind {
    fn from_process_command(command: &str) -> Option<Self> {
        match Path::new(command)
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.trim_start_matches('-'))
        {
            Some("fish") => Some(Self::Fish),
            Some("bash") => Some(Self::Bash),
            Some("zsh") => Some(Self::Zsh),
            Some("powershell") | Some("powershell.exe") | Some("pwsh") | Some("pwsh.exe") => {
                Some(Self::PowerShell)
            }
            _ => None,
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        match name {
            "fish" => Some(Self::Fish),
            "bash" => Some(Self::Bash),
            "zsh" => Some(Self::Zsh),
            "powershell" => Some(Self::PowerShell),
            _ => None,
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::Fish => "Fish",
            Self::Bash => "Bash",
            Self::Zsh => "Zsh",
            Self::PowerShell => "PowerShell",
        }
    }

    fn legacy_integration_command(self) -> &'static str {
        match self {
            Self::Fish => "qn shell-init fish | source",
            Self::Bash => "eval \"$(qn shell-init bash)\"",
            Self::Zsh => "eval \"$(qn shell-init zsh)\"",
            Self::PowerShell => "qn shell-init powershell | Invoke-Expression",
        }
    }

    fn init_script(self) -> &'static str {
        match self {
            Self::Fish => FISH_SHELL_INIT,
            Self::Bash => BASH_SHELL_INIT,
            Self::Zsh => ZSH_SHELL_INIT,
            Self::PowerShell => POWERSHELL_SHELL_INIT,
        }
    }

    fn startup_config_path(self) -> Result<PathBuf, String> {
        let home = || {
            env::var_os("HOME")
                .map(PathBuf::from)
                .ok_or_else(|| "无法确定用户目录，请设置 HOME".to_owned())
        };
        match self {
            Self::Fish => Ok(home()?.join(".config/fish/config.fish")),
            Self::Bash => Ok(home()?.join(".bashrc")),
            Self::Zsh => Ok(home()?.join(".zshrc")),
            Self::PowerShell => powershell_profile_path(),
        }
    }

    fn install_command(self) -> &'static str {
        match self {
            Self::Fish => "qn init-shell fish",
            Self::Bash => "qn init-shell bash",
            Self::Zsh => "qn init-shell zsh",
            Self::PowerShell => "qn init-shell powershell",
        }
    }
}

#[cfg(windows)]
fn powershell_profile_path() -> Result<PathBuf, String> {
    let output = Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "[Console]::Out.Write($PROFILE.CurrentUserCurrentHost)",
        ])
        .output()
        .map_err(|error| format!("无法启动 PowerShell：{error}"))?;
    if !output.status.success() {
        return Err(format!(
            "无法确定 PowerShell Profile 路径：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let path = String::from_utf8(output.stdout)
        .map_err(|error| format!("PowerShell Profile 路径不是有效 UTF-8：{error}"))?;
    let path = path.trim();
    if path.is_empty() {
        return Err("PowerShell 未返回 Profile 路径".into());
    }
    Ok(PathBuf::from(path))
}

#[cfg(not(windows))]
fn powershell_profile_path() -> Result<PathBuf, String> {
    Err("PowerShell 集成仅支持 Windows".into())
}

const FISH_SHELL_INIT: &str = r#"function qn
    set -lx QN_SHELL_INTEGRATION 1

    if test (count $argv) -gt 0
        switch $argv[1]
            case -h --help -t --text -m --markdown -i --image -f --file --status init init-shell __notify __is-configured
                command qn $argv
                return $status
        end
    end

    set -l qn_original $argv
    set -l qn_notify 1
    set -l qn_attach 0
    set -l qn_has_shell_script 0
    set -l qn_shell_script
    set -l qn_args $argv
    while test (count $qn_args) -gt 0
        switch $qn_args[1]
            case --no-notify
                set qn_notify 0
                set qn_args $qn_args[2..-1]
            case -a --attach-output
                set qn_attach 1
                set qn_args $qn_args[2..-1]
            case --shell
                if test (count $qn_args) -lt 2
                    command qn $qn_original
                    return $status
                end
                set qn_has_shell_script 1
                set qn_shell_script $qn_args[2]
                set qn_args $qn_args[3..-1]
                if test (count $qn_args) -gt 0
                    command qn $qn_original
                    return $status
                end
                break
            case --
                set qn_args $qn_args[2..-1]
                break
            case '-*'
                command qn $qn_original
                return $status
            case '*'
                break
        end
    end

    if test $qn_has_shell_script -eq 0; and test (count $qn_args) -eq 0
        command qn $qn_original
        return $status
    end

    if test $qn_notify -eq 1; and not set -q __qn_config_checked
        set -g __qn_config_checked 1
        if not command qn __is-configured
            printf '%s\n' '提示：qn 尚未初始化；运行 `qn init` 完成通知配置。本次命令会照常执行，但不会发送通知。' >&2
        end
    end

    set -l qn_started (date +%s)
    set -l qn_display
    if test $qn_has_shell_script -eq 1
        set qn_display "shell: $qn_shell_script"
    else
        set qn_display (string join -- ' ' (string escape -- $qn_args))
    end

    set -l qn_tempdir
    set -l qn_stdout
    set -l qn_stderr
    set -l qn_status
    if test $qn_attach -eq 1
        set qn_tempdir (mktemp -d)
        or begin
            printf '%s\n' '错误：无法创建 qn 输出临时目录。' >&2
            return 1
        end
        set qn_stdout "$qn_tempdir/stdout"
        set qn_stderr "$qn_tempdir/stderr"
        touch "$qn_stdout" "$qn_stderr"
        or begin
            printf '%s\n' '错误：无法创建 qn 输出临时文件。' >&2
            command rm -rf -- "$qn_tempdir"
            return 1
        end
        if test $qn_has_shell_script -eq 1
            eval $qn_shell_script >"$qn_stdout" 2>"$qn_stderr"
        else
            $qn_args >"$qn_stdout" 2>"$qn_stderr"
        end
        set qn_status $status
        cat "$qn_stdout"
        cat "$qn_stderr" >&2
    else
        if test $qn_has_shell_script -eq 1
            eval $qn_shell_script
        else
            $qn_args
        end
        set qn_status $status
    end

    set -l qn_elapsed (math (date +%s) - $qn_started)
    if test $qn_notify -eq 1
        set -l qn_report __notify --command "$qn_display" --exit-code "$qn_status" --elapsed "$qn_elapsed"
        if test $qn_attach -eq 1
            set -a qn_report --stdout-file "$qn_stdout" --stderr-file "$qn_stderr"
        end
        command qn $qn_report
    end
    if test $qn_attach -eq 1
        command rm -rf -- "$qn_tempdir"
    end
    return $qn_status
end
"#;

const BASH_SHELL_INIT: &str = r#"qn() {
    local QN_SHELL_INTEGRATION=1
    export QN_SHELL_INTEGRATION

    case "${1-}" in
        -h|--help|-t|--text|-m|--markdown|-i|--image|-f|--file|--status|init|init-shell|__notify|__is-configured)
            command qn "$@"
            return $?
            ;;
    esac

    local -a qn_original=("$@") qn_args qn_report
    local qn_notify=1 qn_attach=0 qn_has_shell_script=0 qn_shell_script
    while (($#)); do
        case "$1" in
            --no-notify) qn_notify=0; shift ;;
            -a|--attach-output) qn_attach=1; shift ;;
            --shell)
                if (($# < 2)); then
                    command qn "${qn_original[@]}"
                    return $?
                fi
                qn_has_shell_script=1
                qn_shell_script=$2
                shift 2
                if (($#)); then
                    command qn "${qn_original[@]}"
                    return $?
                fi
                break
                ;;
            --) shift; break ;;
            -?*) command qn "${qn_original[@]}"; return $? ;;
            *) break ;;
        esac
    done
    qn_args=("$@")
    if (( ! qn_has_shell_script && ${#qn_args[@]} == 0 )); then
        command qn "${qn_original[@]}"
        return $?
    fi

    if (( qn_notify )) && [[ -z ${__qn_config_checked+x} ]]; then
        __qn_config_checked=1
        if ! command qn __is-configured; then
            printf '%s\n' '提示：qn 尚未初始化；运行 `qn init` 完成通知配置。本次命令会照常执行，但不会发送通知。' >&2
        fi
    fi

    local qn_started=$SECONDS qn_display qn_eval qn_status qn_elapsed qn_tempdir qn_stdout qn_stderr
    if (( qn_has_shell_script )); then
        qn_display="shell: $qn_shell_script"
        qn_eval=$qn_shell_script
    else
        printf -v qn_display '%q ' "${qn_args[@]}"
        qn_display=${qn_display% }
        qn_eval=$qn_display
    fi
    if (( qn_attach )); then
        qn_tempdir=$(mktemp -d "${TMPDIR:-/tmp}/qn.XXXXXX") || {
            printf '%s\n' '错误：无法创建 qn 输出临时目录。' >&2
            return 1
        }
        qn_stdout="$qn_tempdir/stdout"
        qn_stderr="$qn_tempdir/stderr"
        if eval "$qn_eval" >"$qn_stdout" 2>"$qn_stderr"; then qn_status=0; else qn_status=$?; fi
        cat "$qn_stdout"
        cat "$qn_stderr" >&2
    else
        if eval "$qn_eval"; then qn_status=0; else qn_status=$?; fi
    fi
    qn_elapsed=$((SECONDS - qn_started))
    if (( qn_notify )); then
        qn_report=(__notify --command "$qn_display" --exit-code "$qn_status" --elapsed "$qn_elapsed")
        if (( qn_attach )); then qn_report+=(--stdout-file "$qn_stdout" --stderr-file "$qn_stderr"); fi
        command qn "${qn_report[@]}" || :
    fi
    if (( qn_attach )); then command rm -rf -- "$qn_tempdir"; fi
    return "$qn_status"
}
"#;

const ZSH_SHELL_INIT: &str = r#"qn() {
    emulate -L zsh
    local QN_SHELL_INTEGRATION=1
    export QN_SHELL_INTEGRATION

    case "${1-}" in
        -h|--help|-t|--text|-m|--markdown|-i|--image|-f|--file|--status|init|init-shell|__notify|__is-configured)
            command qn "$@"
            return $?
            ;;
    esac

    local -a qn_original qn_args qn_report
    qn_original=("$@")
    local qn_notify=1 qn_attach=0 qn_has_shell_script=0 qn_shell_script
    while (($#)); do
        case "$1" in
            --no-notify) qn_notify=0; shift ;;
            -a|--attach-output) qn_attach=1; shift ;;
            --shell)
                if (($# < 2)); then
                    command qn "${qn_original[@]}"
                    return $?
                fi
                qn_has_shell_script=1
                qn_shell_script=$2
                shift 2
                if (($#)); then
                    command qn "${qn_original[@]}"
                    return $?
                fi
                break
                ;;
            --) shift; break ;;
            -?*) command qn "${qn_original[@]}"; return $? ;;
            *) break ;;
        esac
    done
    qn_args=("$@")
    if (( ! qn_has_shell_script && ${#qn_args[@]} == 0 )); then
        command qn "${qn_original[@]}"
        return $?
    fi

    if (( qn_notify )) && [[ -z ${__qn_config_checked+x} ]]; then
        typeset -g __qn_config_checked=1
        if ! command qn __is-configured; then
            printf '%s\n' '提示：qn 尚未初始化；运行 `qn init` 完成通知配置。本次命令会照常执行，但不会发送通知。' >&2
        fi
    fi

    local qn_started=$SECONDS qn_display qn_eval qn_status qn_elapsed qn_tempdir qn_stdout qn_stderr
    if (( qn_has_shell_script )); then
        qn_display="shell: $qn_shell_script"
        qn_eval=$qn_shell_script
    else
        qn_eval="${(j: :)${(@q)qn_args}}"
        qn_display=$qn_eval
    fi
    if (( qn_attach )); then
        qn_tempdir=$(mktemp -d "${TMPDIR:-/tmp}/qn.XXXXXX") || {
            printf '%s\n' '错误：无法创建 qn 输出临时目录。' >&2
            return 1
        }
        qn_stdout="$qn_tempdir/stdout"
        qn_stderr="$qn_tempdir/stderr"
        if eval "$qn_eval" >"$qn_stdout" 2>"$qn_stderr"; then qn_status=0; else qn_status=$?; fi
        cat "$qn_stdout"
        cat "$qn_stderr" >&2
    else
        if eval "$qn_eval"; then qn_status=0; else qn_status=$?; fi
    fi
    qn_elapsed=$(( SECONDS - qn_started ))
    qn_elapsed=${qn_elapsed%.*}
    if (( qn_notify )); then
        qn_report=(__notify --command "$qn_display" --exit-code "$qn_status" --elapsed "$qn_elapsed")
        if (( qn_attach )); then qn_report+=(--stdout-file "$qn_stdout" --stderr-file "$qn_stderr"); fi
        command qn "${qn_report[@]}" || :
    fi
    if (( qn_attach )); then command rm -rf -- "$qn_tempdir"; fi
    return "$qn_status"
}
"#;

const POWERSHELL_SHELL_INIT: &str = r#"function qn {
    $env:QN_SHELL_INTEGRATION = "1"
    $qn_binary = (Get-Command qn -CommandType Application -ErrorAction Stop | Select-Object -First 1).Path
    $qn_original = @($args)
    $qn_passthrough = @("-h", "--help", "-t", "--text", "-m", "--markdown", "-i", "--image", "-f", "--file", "--status", "init", "init-shell", "__notify", "__is-configured")

    if ($qn_original.Count -gt 0 -and $qn_original[0] -in $qn_passthrough) {
        & $qn_binary @qn_original
        return
    }

    $qn_args = [System.Collections.Generic.List[string]]::new()
    foreach ($qn_argument in $qn_original) {
        [void]$qn_args.Add([string]$qn_argument)
    }
    $qn_notify = $true
    $qn_attach = $false
    $qn_has_shell_script = $false
    $qn_shell_script = $null

    while ($qn_args.Count -gt 0) {
        $qn_argument = $qn_args[0]
        if ($qn_argument -eq "--no-notify") {
            $qn_notify = $false
            $qn_args.RemoveAt(0)
        } elseif ($qn_argument -eq "-a" -or $qn_argument -eq "--attach-output") {
            $qn_attach = $true
            $qn_args.RemoveAt(0)
        } elseif ($qn_argument -eq "--shell") {
            if ($qn_args.Count -ne 2) {
                & $qn_binary @qn_original
                return
            }
            $qn_has_shell_script = $true
            $qn_shell_script = $qn_args[1]
            $qn_args.Clear()
        } elseif ($qn_argument -eq "--") {
            $qn_args.RemoveAt(0)
            break
        } elseif ($qn_argument.StartsWith("-")) {
            & $qn_binary @qn_original
            return
        } else {
            break
        }
    }

    if (-not $qn_has_shell_script -and $qn_args.Count -eq 0) {
        & $qn_binary @qn_original
        return
    }

    if ($qn_notify -and -not $script:__qn_config_checked) {
        $script:__qn_config_checked = $true
        & $qn_binary __is-configured
        if ($LASTEXITCODE -ne 0) {
            [Console]::Error.WriteLine("提示：qn 尚未初始化；运行 qn init 完成通知配置。本次命令会照常执行，但不会发送通知。")
        }
    }

    $qn_started = Get-Date
    if ($qn_has_shell_script) {
        $qn_display = "shell: $qn_shell_script"
    } else {
        $qn_display = $qn_args.ToArray() -join " "
    }

    $qn_tempdir = $null
    $qn_stdout = $null
    $qn_stderr = $null
    $qn_output_encoding = if ($PSVersionTable.PSVersion.Major -lt 6) { "utf-16le" } else { "auto" }
    $global:LASTEXITCODE = 0
    if ($qn_attach) {
        $qn_tempdir = Join-Path ([System.IO.Path]::GetTempPath()) ("qn-" + [guid]::NewGuid().ToString("N"))
        [System.IO.Directory]::CreateDirectory($qn_tempdir) | Out-Null
        $qn_stdout = Join-Path $qn_tempdir "stdout"
        $qn_stderr = Join-Path $qn_tempdir "stderr"
        if ($qn_has_shell_script) {
            Invoke-Expression $qn_shell_script 1> $qn_stdout 2> $qn_stderr
        } else {
            $qn_command = $qn_args[0]
            $qn_command_args = if ($qn_args.Count -gt 1) { $qn_args.GetRange(1, $qn_args.Count - 1).ToArray() } else { @() }
            & $qn_command @qn_command_args 1> $qn_stdout 2> $qn_stderr
        }
        $qn_success = $?
        $qn_stdout_content = Get-Content -LiteralPath $qn_stdout -Raw -ErrorAction Ignore
        if ($null -ne $qn_stdout_content) {
            Write-Host -NoNewline $qn_stdout_content
        }
        $qn_stderr_content = Get-Content -LiteralPath $qn_stderr -Raw -ErrorAction Ignore
        if ($null -ne $qn_stderr_content) {
            [Console]::Error.Write($qn_stderr_content)
        }
    } elseif ($qn_has_shell_script) {
        Invoke-Expression $qn_shell_script
        $qn_success = $?
    } else {
        $qn_command = $qn_args[0]
        $qn_command_args = if ($qn_args.Count -gt 1) { $qn_args.GetRange(1, $qn_args.Count - 1).ToArray() } else { @() }
        & $qn_command @qn_command_args
        $qn_success = $?
    }

    if ($qn_success) {
        $qn_status = 0
    } elseif ($LASTEXITCODE -ne 0) {
        $qn_status = [int]$LASTEXITCODE
    } else {
        $qn_status = 1
    }
    $qn_elapsed = [math]::Floor(((Get-Date) - $qn_started).TotalSeconds)
    if ($qn_notify) {
        $qn_report = @("__notify", "--command", $qn_display, "--exit-code", "$qn_status", "--elapsed", "$qn_elapsed")
        if ($qn_attach) {
            $qn_report += @("--stdout-file", $qn_stdout, "--stderr-file", $qn_stderr, "--output-encoding", $qn_output_encoding)
        }
        & $qn_binary @qn_report | Out-Null
    }
    if ($qn_attach) {
        Remove-Item -LiteralPath $qn_tempdir -Recurse -Force
    }
    $global:LASTEXITCODE = $qn_status
}
"#;
#[derive(Debug)]
enum Invocation {
    Run(Options),
    Text(String),
    Markdown(String),
    Media {
        path: PathBuf,
        media_type: MediaType,
    },
    Status,
}

#[derive(Debug)]
struct Options {
    command: Vec<String>,
    shell: Option<String>,
    notify: bool,
    attach_output: bool,
}

#[derive(Debug)]
struct CommandResult {
    code: i32,
    output: Option<std::process::Output>,
}

#[derive(Debug, PartialEq, Eq)]
struct ShellReport {
    command: String,
    code: i32,
    elapsed_seconds: u64,
    stdout_file: Option<PathBuf>,
    stderr_file: Option<PathBuf>,
    output_encoding: CapturedOutputEncoding,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum CapturedOutputEncoding {
    #[default]
    Auto,
    Utf16Le,
    Utf16Be,
}

impl CapturedOutputEncoding {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "auto" => Ok(Self::Auto),
            "utf-16le" => Ok(Self::Utf16Le),
            "utf-16be" => Ok(Self::Utf16Be),
            _ => Err(format!("不支持的输出编码：{value}")),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct CommandNotification {
    markdown: String,
    output_attachment: Option<String>,
}

#[derive(Serialize)]
struct Notification<'a> {
    summary: &'a str,
}

#[derive(Serialize)]
struct ApiMessage<'a> {
    content: &'a str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MediaType {
    Image,
    File,
}

impl MediaType {
    fn api_value(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::File => "file",
        }
    }
}

#[derive(Deserialize)]
struct StatusResponse {
    connected: bool,
    bound: bool,
}

fn config_path() -> Result<PathBuf, String> {
    let base_dirs = BaseDirs::new().ok_or("无法确定当前用户的配置目录")?;
    Ok(base_dirs.config_dir().join("qn").join("config"))
}

fn read_config_value_from(path: &Path, name: &str) -> Option<String> {
    let contents = fs::read_to_string(path).ok()?;
    contents
        .lines()
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            (key.trim() == name).then(|| value.trim().to_owned())
        })
        .last()
}

fn read_config_value(name: &str) -> Option<String> {
    read_config_value_from(&config_path().ok()?, name)
}

fn configured_value(environment_name: &str, config_name: &str) -> Option<String> {
    env::var(environment_name)
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| read_config_value(config_name).filter(|value| !value.is_empty()))
}

fn is_configured() -> bool {
    configured_value("QN_TOKEN", "token").is_some()
        && configured_value("QN_ENDPOINT", "endpoint").is_some()
}

fn api_url_from_root(root: &str, path: &str) -> String {
    format!("{}/{path}", root.trim_end_matches('/'))
}

fn api_url(path: &str) -> Result<String, String> {
    let endpoint = configured_value("QN_ENDPOINT", "endpoint")
        .ok_or("未配置 QN_ENDPOINT，请运行 `qn init` 或设置环境变量".to_owned())?;
    Ok(api_url_from_root(&endpoint, path))
}

fn shell_integration_is_loaded() -> bool {
    env::var(SHELL_INTEGRATION_ENV).is_ok_and(|value| value == "1")
}

#[cfg(unix)]
fn process_info(pid: u32) -> Option<(u32, String)> {
    let output = Command::new("/bin/ps")
        .args(["-p", &pid.to_string(), "-o", "ppid=", "-o", "comm="])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut fields = stdout.split_whitespace();
    let parent_pid = fields.next()?.parse().ok()?;
    let command = fields.next()?.to_owned();
    Some((parent_pid, command))
}

#[cfg(unix)]
fn shell_from_ancestor_processes() -> Option<ShellKind> {
    let mut pid = std::process::id();
    for _ in 0..8 {
        let (parent_pid, command) = process_info(pid)?;
        if let Some(shell) = ShellKind::from_process_command(&command) {
            return Some(shell);
        }
        if parent_pid <= 1 || parent_pid == pid {
            break;
        }
        pid = parent_pid;
    }
    None
}

#[cfg(windows)]
fn shell_from_ancestor_processes() -> Option<ShellKind> {
    env::var_os("PSModulePath").map(|_| ShellKind::PowerShell)
}

#[cfg(all(not(unix), not(windows)))]
fn shell_from_ancestor_processes() -> Option<ShellKind> {
    None
}

fn warn_missing_shell_integration() {
    if let Some(shell) = shell_from_ancestor_processes() {
        eprintln!(
            "提示：当前 {} 未加载 qn 集成；运行 `{}` 会将集成写入启动配置，之后 qn 才能使用 alias 和 function。",
            shell.display_name(),
            shell.install_command()
        );
    } else {
        #[cfg(windows)]
        eprintln!(
            "提示：当前 PowerShell 未加载 qn 集成；运行 `{}` 会将集成写入启动配置。",
            ShellKind::PowerShell.install_command()
        );
        #[cfg(not(windows))]
        eprintln!(
            "提示：当前 Shell 未加载 qn 集成。运行其一以写入启动配置：\n  Fish: {}\n  Bash: {}\n  Zsh:  {}",
            ShellKind::Fish.install_command(),
            ShellKind::Bash.install_command(),
            ShellKind::Zsh.install_command()
        );
    }
}

const SHELL_INTEGRATION_BEGIN_MARKER: &str = "# >>> qn shell integration >>>";
const SHELL_INTEGRATION_END_MARKER: &str = "# <<< qn shell integration <<<";

fn shell_integration_uses_utf8_bom(shell: ShellKind) -> bool {
    #[cfg(windows)]
    {
        shell == ShellKind::PowerShell
    }
    #[cfg(not(windows))]
    {
        let _ = shell;
        false
    }
}

fn write_shell_integration_contents(
    path: &Path,
    contents: &str,
    shell: ShellKind,
) -> Result<(), String> {
    if shell_integration_uses_utf8_bom(shell) {
        let mut bytes = Vec::with_capacity(contents.len() + 3);
        bytes.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
        bytes.extend_from_slice(contents.as_bytes());
        fs::write(path, bytes)
    } else {
        fs::write(path, contents)
    }
    .map_err(|error| format!("写入 Shell 启动配置失败（{}）: {error}", path.display()))
}

fn write_shell_integration(path: &Path, shell: ShellKind) -> Result<bool, String> {
    let mut contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(format!(
                "读取 Shell 启动配置失败（{}）: {error}",
                path.display()
            ));
        }
    };
    let had_utf8_bom = contents.starts_with('\u{feff}');
    if had_utf8_bom {
        contents.remove(0);
    }
    let legacy_command = shell.legacy_integration_command();
    let without_legacy_loader = contents
        .split_inclusive('\n')
        .filter(|line| line.trim() != legacy_command)
        .collect::<String>();
    let mut updated = if without_legacy_loader == contents {
        contents.clone()
    } else {
        without_legacy_loader
    };

    let integration = format!(
        "{SHELL_INTEGRATION_BEGIN_MARKER}\n{}{SHELL_INTEGRATION_END_MARKER}\n",
        shell.init_script()
    );
    if let Some(begin) = updated.find(SHELL_INTEGRATION_BEGIN_MARKER) {
        let end = updated[begin..]
            .find(SHELL_INTEGRATION_END_MARKER)
            .map(|offset| begin + offset + SHELL_INTEGRATION_END_MARKER.len())
            .ok_or_else(|| format!("Shell 启动配置中的 qn 集成标记不完整（{}）", path.display()))?;
        let end = if updated[end..].starts_with('\n') {
            end + 1
        } else {
            end
        };
        updated.replace_range(begin..end, &integration);
    } else {
        if !updated.is_empty() && !updated.ends_with('\n') {
            updated.push('\n');
        }
        updated.push_str(&integration);
    }

    if updated == contents && (!shell_integration_uses_utf8_bom(shell) || had_utf8_bom) {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("创建 Shell 配置目录失败（{}）: {error}", parent.display()))?;
    }
    let is_new = !path.exists();
    write_shell_integration_contents(path, &updated, shell)?;
    #[cfg(unix)]
    if is_new {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("设置配置权限失败: {error}"))?;
    }
    Ok(true)
}

fn install_shell_integration(shell_name: &str) -> Result<String, String> {
    let shell = ShellKind::from_name(shell_name).ok_or_else(|| {
        format!("不支持的 Shell：{shell_name}；请使用 fish、bash、zsh 或 powershell")
    })?;
    let path = shell.startup_config_path()?;
    let updated = write_shell_integration(&path, shell)?;
    let state = if updated {
        "已写入"
    } else {
        "已经是最新内容"
    };
    Ok(format!(
        "qn 集成{} {}；重启 {} 后生效。",
        state,
        path.display(),
        shell.display_name()
    ))
}
fn write_config_value(path: &Path, name: &str, value: &str) -> Result<(), String> {
    if value.contains(['\n', '\r']) {
        return Err("配置值不能包含换行符".into());
    }

    let add_leading_newline = match fs::read_to_string(path) {
        Ok(contents) => !contents.is_empty() && !contents.ends_with('\n'),
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => return Err(format!("读取配置失败: {error}")),
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("创建配置目录失败: {error}"))?;
    }
    let is_new = !path.exists();
    let mut file = OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)
        .map_err(|error| format!("写入配置失败: {error}"))?;
    if add_leading_newline {
        writeln!(file).map_err(|error| format!("写入配置失败: {error}"))?;
    }
    writeln!(file, "{name}={value}").map_err(|error| format!("写入配置失败: {error}"))?;
    #[cfg(unix)]
    if is_new {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("设置配置权限失败: {error}"))?;
    }
    Ok(())
}

fn macos_local_hostname() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let output = Command::new("scutil")
            .args(["--get", "LocalHostName"])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        return String::from_utf8(output.stdout)
            .ok()
            .map(|name| name.trim().to_owned())
            .filter(|name| !name.is_empty());
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

fn select_default_device_name(
    hostname: String,
    local_hostname: Option<String>,
) -> Result<String, String> {
    let hostname = hostname.trim().to_owned();
    if !hostname.is_empty() && !hostname.eq_ignore_ascii_case("localhost") {
        return Ok(hostname);
    }
    if let Some(local_hostname) = local_hostname
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty() && !name.eq_ignore_ascii_case("localhost"))
    {
        return Ok(local_hostname);
    }
    if hostname.is_empty() {
        Err("主机名为空，无法设置设备名称".into())
    } else {
        Ok(hostname)
    }
}

fn default_device_name() -> Result<String, String> {
    let hostname = hostname::get()
        .map_err(|error| format!("获取主机名失败: {error}"))?
        .to_string_lossy()
        .into_owned();
    select_default_device_name(hostname, macos_local_hostname())
}

fn device_name_from_config(path: &Path) -> Result<String, String> {
    if let Some(name) = read_config_value_from(path, "name").filter(|name| !name.is_empty()) {
        return Ok(name);
    }

    let name = default_device_name()?;
    write_config_value(path, "name", &name)?;
    Ok(name)
}

fn device_name() -> Result<String, String> {
    device_name_from_config(&config_path()?)
}

fn initialize_config_with_io(
    path: &Path,
    endpoint: Option<String>,
    input: &mut impl BufRead,
    output: &mut impl Write,
    device_name: impl FnOnce() -> Result<String, String>,
) -> Result<(), String> {
    let endpoint = match endpoint {
        Some(endpoint) => endpoint,
        None => {
            writeln!(output, "请输入 QN_ENDPOINT（默认 {DEFAULT_ENDPOINT}）：")
                .map_err(|error| error.to_string())?;
            output.flush().map_err(|error| error.to_string())?;
            let mut ep = String::new();
            input
                .read_line(&mut ep)
                .map_err(|error| error.to_string())?;
            let ep = ep.trim();
            if ep.is_empty() {
                DEFAULT_ENDPOINT.to_owned()
            } else {
                ep.to_owned()
            }
        }
    };
    writeln!(output, "请输入 QN_TOKEN：").map_err(|error| error.to_string())?;
    output.flush().map_err(|error| error.to_string())?;
    let mut token = String::new();
    input
        .read_line(&mut token)
        .map_err(|error| error.to_string())?;
    let token = token.trim();
    if token.is_empty() {
        return Err("QN_TOKEN 不能为空".into());
    }
    let default_name = device_name()?;
    writeln!(output, "请输入设备名称（默认 {default_name}）：")
        .map_err(|error| error.to_string())?;
    output.flush().map_err(|error| error.to_string())?;
    let mut name = String::new();
    input
        .read_line(&mut name)
        .map_err(|error| error.to_string())?;
    let name = if name.trim().is_empty() {
        default_name
    } else {
        name.trim().to_owned()
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let config = format!("endpoint={endpoint}\ntoken={token}\nname={name}\n");
    fs::write(path, config).map_err(|error| format!("写入配置失败: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())?;
    }
    writeln!(output, "配置已保存：{}", path.display()).map_err(|error| error.to_string())?;
    writeln!(
        output,
        "要让 qn 使用当前 Shell 的 alias 和 function，请运行对应命令写入启动配置："
    )
    .map_err(|error| error.to_string())?;
    #[cfg(windows)]
    writeln!(output, "  powershell: qn init-shell powershell")
        .map_err(|error| error.to_string())?;
    #[cfg(not(windows))]
    {
        writeln!(output, "  fish: qn init-shell fish").map_err(|error| error.to_string())?;
        writeln!(output, "  bash: qn init-shell bash").map_err(|error| error.to_string())?;
        writeln!(output, "  zsh:  qn init-shell zsh").map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[cfg(unix)]
fn initialize_config() -> Result<(), String> {
    let path = config_path()?;
    let tty = fs::File::options()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .map_err(|_| "配置需要交互式终端；请在终端中运行 `qn init`".to_owned())?;
    let mut input = io::BufReader::new(tty.try_clone().map_err(|error| error.to_string())?);
    let mut output = tty;
    initialize_config_with_io(
        &path,
        env::var("QN_ENDPOINT").ok(),
        &mut input,
        &mut output,
        default_device_name,
    )
}

#[cfg(not(unix))]
fn initialize_config() -> Result<(), String> {
    let path = config_path()?;
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut input = stdin.lock();
    let mut output = stdout.lock();
    initialize_config_with_io(
        &path,
        env::var("QN_ENDPOINT").ok(),
        &mut input,
        &mut output,
        default_device_name,
    )
}

fn print_usage() {
    eprintln!("用法:");
    eprintln!("  qn [-a|--attach-output] [--no-notify] <command> [args...]");
    eprintln!("  qn [-a|--attach-output] [--no-notify] --shell <command-string>");
    eprintln!("  qn -t|--text <content>");
    eprintln!("  qn -m|--markdown <content>");
    eprintln!("  qn -i|--image <path>");
    eprintln!("  qn -f|--file <path>");
    eprintln!("  qn --status");
    eprintln!("  qn init");
    eprintln!("  qn init-shell <fish|bash|zsh|powershell>");
    eprintln!();
    eprintln!("选项:");
    eprintln!("  -a, --attach-output  在通知中附带命令的标准输出和标准错误");
    eprintln!("  --no-notify          不发送完成通知");
    eprintln!("  -t, --text           直接发送纯文本消息");
    eprintln!("  -m, --markdown       直接发送 Markdown 消息");
    eprintln!("  -i, --image          上传并以图片消息发送");
    eprintln!("  -f, --file           上传并以文件附件发送");
    eprintln!("  --status             查看 QQ Gateway 与默认接收人状态");
}

fn direct_options_are_allowed(
    notify: bool,
    attach_output: bool,
    shell: Option<&String>,
) -> Result<(), String> {
    if !notify || attach_output || shell.is_some() {
        Err("直接发送选项不能与命令执行选项同时使用".into())
    } else {
        Ok(())
    }
}

fn parse_direct_arguments(
    mut args: impl Iterator<Item = String>,
    value_name: &str,
) -> Result<String, String> {
    let value = args.next().ok_or_else(|| format!("{value_name}不能为空"))?;
    let value = if value == "--" {
        args.next().ok_or_else(|| format!("{value_name}不能为空"))?
    } else {
        value
    };
    if value.is_empty() {
        return Err(format!("{value_name}不能为空"));
    }
    if let Some(argument) = args.next() {
        if argument == "--to" {
            return Err("当前服务只支持一个绑定接收人，不能指定 --to".into());
        }
        return Err(format!("只能指定一个{value_name}"));
    }
    Ok(value)
}

fn parse_options() -> Result<Invocation, String> {
    parse_options_from(env::args().skip(1))
}

fn parse_options_from(args: impl IntoIterator<Item = String>) -> Result<Invocation, String> {
    let mut args = args.into_iter();
    let mut notify = true;
    let mut attach_output = false;
    let mut shell = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print_usage();
                std::process::exit(0);
            }
            "--no-notify" => notify = false,
            "-a" | "--attach-output" => attach_output = true,
            "--shell" => {
                let script = args.next().ok_or("--shell 后需要命令字符串")?;
                if args.next().is_some() {
                    return Err("--shell 不能与命令参数同时使用".into());
                }
                shell = Some(script);
                break;
            }
            "-t" | "--text" => {
                direct_options_are_allowed(notify, attach_output, shell.as_ref())?;
                return Ok(Invocation::Text(parse_direct_arguments(args, "消息内容")?));
            }
            "-m" | "--markdown" => {
                direct_options_are_allowed(notify, attach_output, shell.as_ref())?;
                return Ok(Invocation::Markdown(parse_direct_arguments(
                    args,
                    "Markdown 内容",
                )?));
            }
            "-i" | "--image" => {
                direct_options_are_allowed(notify, attach_output, shell.as_ref())?;
                return Ok(Invocation::Media {
                    path: PathBuf::from(parse_direct_arguments(args, "图片路径")?),
                    media_type: MediaType::Image,
                });
            }
            "-f" | "--file" => {
                direct_options_are_allowed(notify, attach_output, shell.as_ref())?;
                return Ok(Invocation::Media {
                    path: PathBuf::from(parse_direct_arguments(args, "文件路径")?),
                    media_type: MediaType::File,
                });
            }
            "--status" => {
                direct_options_are_allowed(notify, attach_output, shell.as_ref())?;
                if args.next().is_some() {
                    return Err("--status 不能指定其他参数".into());
                }
                return Ok(Invocation::Status);
            }
            "--" => {
                let command: Vec<_> = args.collect();
                if command.is_empty() {
                    return Err("`--` 后需要指定要执行的命令".into());
                }
                return Ok(Invocation::Run(Options {
                    command,
                    shell,
                    notify,
                    attach_output,
                }));
            }
            _ if arg.starts_with('-') => return Err(format!("未知 qn 选项：{arg}")),
            _ => {
                let mut command = vec![arg];
                command.extend(args);
                return Ok(Invocation::Run(Options {
                    command,
                    shell,
                    notify,
                    attach_output,
                }));
            }
        }
    }

    if let Some(shell) = shell {
        return Ok(Invocation::Run(Options {
            command: Vec::new(),
            shell: Some(shell),
            notify,
            attach_output,
        }));
    }

    Err("没有指定要执行的命令".into())
}

fn parse_shell_report(args: impl IntoIterator<Item = String>) -> Result<ShellReport, String> {
    let mut args = args.into_iter();
    let mut command = None;
    let mut code = None;
    let mut elapsed_seconds = None;
    let mut stdout_file = None;
    let mut stderr_file = None;
    let mut output_encoding = None;

    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--command" if command.is_none() => {
                command = Some(args.next().ok_or("--command 后需要一个值")?);
            }
            "--exit-code" if code.is_none() => {
                let value = args.next().ok_or("--exit-code 后需要一个值")?;
                code = Some(
                    value
                        .parse()
                        .map_err(|_| format!("无效的退出码：{value}"))?,
                );
            }
            "--elapsed" if elapsed_seconds.is_none() => {
                let value = args.next().ok_or("--elapsed 后需要一个值")?;
                elapsed_seconds = Some(value.parse().map_err(|_| format!("无效的耗时：{value}"))?);
            }
            "--stdout-file" if stdout_file.is_none() => {
                stdout_file = Some(PathBuf::from(
                    args.next().ok_or("--stdout-file 后需要一个值")?,
                ));
            }
            "--stderr-file" if stderr_file.is_none() => {
                stderr_file = Some(PathBuf::from(
                    args.next().ok_or("--stderr-file 后需要一个值")?,
                ));
            }
            "--output-encoding" if output_encoding.is_none() => {
                output_encoding = Some(CapturedOutputEncoding::parse(
                    &args.next().ok_or("--output-encoding 后需要一个值")?,
                )?);
            }
            _ => return Err(format!("未知或重复的通知参数：{argument}")),
        }
    }

    let (stdout_file, stderr_file) = match (stdout_file, stderr_file) {
        (Some(stdout_file), Some(stderr_file)) => (Some(stdout_file), Some(stderr_file)),
        (None, None) => (None, None),
        _ => return Err("标准输出和标准错误文件必须同时提供".into()),
    };
    if output_encoding.is_some() && stdout_file.is_none() {
        return Err("--output-encoding 只能与输出文件同时提供".into());
    }
    Ok(ShellReport {
        command: command.ok_or("--command 不能为空")?,
        code: code.ok_or("--exit-code 不能为空")?,
        elapsed_seconds: elapsed_seconds.ok_or("--elapsed 不能为空")?,
        stdout_file,
        stderr_file,
        output_encoding: output_encoding.unwrap_or_default(),
    })
}

fn read_captured_output_file(path: &Path, output_name: &str) -> Result<Vec<u8>, String> {
    match fs::read(path) {
        Ok(output) => Ok(output),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(format!(
            "读取{output_name}失败（{}）: {error}",
            path.display()
        )),
    }
}

fn notify_shell_report(args: impl IntoIterator<Item = String>) -> Result<(), String> {
    let report = parse_shell_report(args)?;
    if !is_configured() {
        return Ok(());
    }
    let output = match (report.stdout_file, report.stderr_file) {
        (Some(stdout_file), Some(stderr_file)) => {
            let stdout = read_captured_output_file(&stdout_file, "标准输出")?;
            let stderr = read_captured_output_file(&stderr_file, "标准错误")?;
            Some(command_output_display_bytes(
                &stdout,
                &stderr,
                report.output_encoding,
            ))
        }
        (None, None) => None,
        _ => unreachable!("parse_shell_report ensures output files are paired"),
    };
    notify_command_report(&report.command, report.code, report.elapsed_seconds, output)
}

fn run(options: &Options) -> Result<CommandResult, String> {
    let mut process = if let Some(script) = &options.shell {
        if cfg!(target_os = "windows") {
            let mut command = Command::new("cmd");
            command.args(["/C", script]);
            command
        } else {
            let mut command = Command::new("sh");
            command.args(["-c", script]);
            command
        }
    } else {
        let mut command = Command::new(&options.command[0]);
        command.args(&options.command[1..]);
        command
    };

    if options.attach_output {
        let output = process
            .output()
            .map_err(|error| format!("启动命令失败: {error}"))?;
        io::stdout()
            .write_all(&output.stdout)
            .map_err(|error| format!("写入命令标准输出失败: {error}"))?;
        io::stderr()
            .write_all(&output.stderr)
            .map_err(|error| format!("写入命令标准错误失败: {error}"))?;
        Ok(CommandResult {
            code: output.status.code().unwrap_or(1),
            output: Some(output),
        })
    } else {
        let status = process
            .status()
            .map_err(|error| format!("启动命令失败: {error}"))?;
        Ok(CommandResult {
            code: status.code().unwrap_or(1),
            output: None,
        })
    }
}

fn command_display(options: &Options) -> String {
    if let Some(script) = &options.shell {
        format!("shell: {script}")
    } else {
        options
            .command
            .iter()
            .map(|arg| shell_quote(arg))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

fn shell_quote(value: &str) -> String {
    if value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "_./:@%+=,-".contains(character))
    {
        value.to_owned()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

fn command_output_display(output: &std::process::Output) -> String {
    command_output_display_bytes(&output.stdout, &output.stderr, CapturedOutputEncoding::Auto)
}

fn decode_with_encoding(bytes: &[u8], encoding: &'static Encoding) -> String {
    let (text, _, _) = encoding.decode(bytes);
    text.into_owned()
}

fn decode_windows_code_page(bytes: &[u8], code_page: u16) -> Option<String> {
    let encoding = codepage::to_encoding(code_page)?;
    let (text, _, had_errors) = encoding.decode(bytes);
    (!had_errors).then(|| text.into_owned())
}

#[cfg(windows)]
fn windows_console_output_code_page() -> Option<u16> {
    use windows_sys::Win32::System::Console::GetConsoleOutputCP;

    u16::try_from(unsafe { GetConsoleOutputCP() }).ok()
}

fn decode_output_bytes(bytes: &[u8], output_encoding: CapturedOutputEncoding) -> String {
    if bytes.is_empty() {
        return String::new();
    }
    match output_encoding {
        CapturedOutputEncoding::Utf16Le => return decode_with_encoding(bytes, UTF_16LE),
        CapturedOutputEncoding::Utf16Be => return decode_with_encoding(bytes, UTF_16BE),
        CapturedOutputEncoding::Auto => {}
    }
    if let Some((encoding, _)) = Encoding::for_bom(bytes) {
        return decode_with_encoding(bytes, encoding);
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        return text.to_owned();
    }
    #[cfg(windows)]
    if let Some(text) = windows_console_output_code_page()
        .and_then(|code_page| decode_windows_code_page(bytes, code_page))
    {
        return text;
    }
    let mut detector = EncodingDetector::new();
    detector.feed(bytes, true);
    decode_with_encoding(bytes, detector.guess(None, true))
}

fn command_output_display_bytes(
    stdout: &[u8],
    stderr: &[u8],
    output_encoding: CapturedOutputEncoding,
) -> String {
    let stdout = decode_output_bytes(stdout, output_encoding);
    let stderr = decode_output_bytes(stderr, output_encoding);
    match (stdout.is_empty(), stderr.is_empty()) {
        (true, true) => "（无输出）".to_owned(),
        (false, true) => format!("标准输出：\n{stdout}"),
        (true, false) => format!("标准错误：\n{stderr}"),
        (false, false) => format!("标准输出：\n{stdout}\n标准错误：\n{stderr}"),
    }
}

const MAX_DEVICE_NAME_CHARS: usize = 64;
const MAX_WORKING_DIRECTORY_CHARS: usize = 256;
const MAX_COMMAND_CHARS: usize = 320;
const MAX_INLINE_OUTPUT_CHARS: usize = 1_000;
const MAX_MARKDOWN_CONTENT_CHARS: usize = 4_000;

fn truncate_text(value: &str, maximum_characters: usize, suffix: &str) -> (String, bool) {
    if value.chars().count() <= maximum_characters {
        return (value.to_owned(), false);
    }
    let prefix_length = maximum_characters.saturating_sub(suffix.chars().count());
    let prefix: String = value.chars().take(prefix_length).collect();
    (format!("{prefix}{suffix}"), true)
}

fn markdown_text(value: &str, maximum_characters: usize) -> String {
    let (value, _) = truncate_text(value, maximum_characters, "…");
    let mut escaped = String::with_capacity(value.len());
    for character in value.replace(['\r', '\n'], " ").chars() {
        if matches!(
            character,
            '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '#' | '+' | '-' | '|'
        ) {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

fn markdown_code_block(language: &str, content: &str) -> String {
    let mut longest_backtick_run = 0;
    let mut current_backtick_run = 0;
    for character in content.chars() {
        if character == '`' {
            current_backtick_run += 1;
            longest_backtick_run = longest_backtick_run.max(current_backtick_run);
        } else {
            current_backtick_run = 0;
        }
    }
    let fence = "`".repeat(longest_backtick_run.max(3) + 1);
    format!("\n{fence}{language}\n{content}\n{fence}")
}

fn build_markdown_notification(
    device_name: &str,
    command: &str,
    code: i32,
    elapsed_seconds: u64,
    working_directory: &str,
    output: Option<String>,
) -> CommandNotification {
    let state = if code == 0 {
        "任务完成"
    } else {
        "任务失败"
    };
    let device_name = markdown_text(device_name, MAX_DEVICE_NAME_CHARS);
    let working_directory = markdown_text(working_directory, MAX_WORKING_DIRECTORY_CHARS);
    let (command, _) = truncate_text(command, MAX_COMMAND_CHARS, "…（已截断）");
    let mut markdown = format!(
        "## {state}\n\n**设备**：{device_name}\n\n**耗时**：{} · **退出码**：{code}\n\n**工作目录**：{working_directory}\n\n### 命令{}",
        format_duration(elapsed_seconds),
        markdown_code_block("sh", &command),
    );
    let output_attachment = output.and_then(|output| {
        if output == "（无输出）" {
            markdown.push_str("\n\n### 输出\n（无输出）");
            return None;
        }
        let (preview, was_truncated) = truncate_text(
            &output,
            MAX_INLINE_OUTPUT_CHARS,
            "\n…（已截断，完整输出见附件）",
        );
        if was_truncated {
            markdown.push_str("\n\n### 输出（已截断，完整日志作为附件发送）");
        } else {
            markdown.push_str("\n\n### 输出");
        }
        markdown.push_str(&markdown_code_block("text", &preview));
        was_truncated.then_some(output)
    });
    debug_assert!(markdown.chars().count() <= MAX_MARKDOWN_CONTENT_CHARS);
    CommandNotification {
        markdown,
        output_attachment,
    }
}

fn notify_command_report(
    command: &str,
    code: i32,
    elapsed_seconds: u64,
    output: Option<String>,
) -> Result<(), String> {
    let working_directory =
        env::current_dir().map_or_else(|_| "未知".into(), |path| path.display().to_string());
    let notification = build_markdown_notification(
        &device_name()?,
        command,
        code,
        elapsed_seconds,
        &working_directory,
        output,
    );
    notify_markdown(&notification.markdown)?;
    if let Some(output) = notification.output_attachment {
        send_output_attachment(output)?;
    }
    Ok(())
}

fn notification_summary(device_name: &str, summary: &str) -> String {
    format!("{device_name}\n{summary}")
}

fn api_token() -> Result<String, String> {
    configured_value("QN_TOKEN", "token")
        .ok_or("未配置 QN_TOKEN，请运行 `qn init` 或设置环境变量".to_owned())
}

fn notification_error(action: &str, error: reqwest::Error) -> String {
    if let Some(status) = error.status() {
        return match status.as_u16() {
            401 | 403 => format!("{action}鉴权失败（HTTP {status}）；请运行 `qn init` 更新配置"),
            404 => format!("{action}的接口不存在（HTTP 404）；请运行 `qn init` 检查 endpoint 配置"),
            412 => format!("{action}失败：默认接收人尚未绑定；请先向机器人发送一条私聊消息"),
            _ => format!("{action}失败（HTTP {status}）：{error}"),
        };
    }
    if error.is_connect() || error.is_timeout() {
        return format!("{action}无法连接通知服务；请检查 endpoint，必要时运行 `qn init`：{error}");
    }
    format!("{action}失败: {error}")
}

fn notify(summary: &str) -> Result<(), String> {
    let token = api_token()?;
    let summary = notification_summary(&device_name()?, summary);
    Client::new()
        .post(api_url("v1/messages")?)
        .bearer_auth(token)
        .json(&Notification { summary: &summary })
        .send()
        .map_err(|error| notification_error("发送通知", error))?
        .error_for_status()
        .map(|_| ())
        .map_err(|error| notification_error("发送通知", error))
}

fn notify_text(content: &str) -> Result<(), String> {
    notify(content)
}

fn notify_markdown(content: &str) -> Result<(), String> {
    Client::new()
        .post(api_url("v1/markdown")?)
        .bearer_auth(api_token()?)
        .json(&ApiMessage { content })
        .send()
        .map_err(|error| notification_error("发送 Markdown 消息", error))?
        .error_for_status()
        .map(|_| ())
        .map_err(|error| notification_error("发送 Markdown 消息", error))
}

fn send_media(path: &Path, media_type: MediaType) -> Result<(), String> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| format!("文件路径必须包含有效文件名：{}", path.display()))?;
    let file = fs::File::open(path)
        .map_err(|error| format!("读取文件失败（{}）: {error}", path.display()))?;
    let part = reqwest::blocking::multipart::Part::reader(file).file_name(file_name.to_owned());
    send_media_part(part, media_type)
}

fn send_output_attachment(output: String) -> Result<(), String> {
    let part = reqwest::blocking::multipart::Part::bytes(output.into_bytes())
        .file_name("qn-output.txt".to_owned());
    send_media_part(part, MediaType::File)
}

fn send_media_part(
    part: reqwest::blocking::multipart::Part,
    media_type: MediaType,
) -> Result<(), String> {
    let form = reqwest::blocking::multipart::Form::new()
        .part("file", part)
        .text("file_type", media_type.api_value().to_owned());
    Client::new()
        .post(api_url("v1/media")?)
        .bearer_auth(api_token()?)
        .multipart(form)
        .send()
        .map_err(|error| notification_error("上传媒体", error))?
        .error_for_status()
        .map(|_| ())
        .map_err(|error| notification_error("上传媒体", error))
}

fn fetch_status() -> Result<StatusResponse, String> {
    Client::new()
        .get(api_url("status")?)
        .bearer_auth(api_token()?)
        .send()
        .map_err(|error| notification_error("查询状态", error))?
        .error_for_status()
        .map_err(|error| notification_error("查询状态", error))?
        .json()
        .map_err(|error| format!("状态接口返回无效 JSON: {error}"))
}

fn main() -> ExitCode {
    let raw_args: Vec<String> = env::args().skip(1).collect();
    if raw_args.len() == 1 && raw_args[0] == "init" {
        return match initialize_config() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("错误: {error}");
                ExitCode::from(1)
            }
        };
    }
    if raw_args
        .first()
        .is_some_and(|argument| argument == "init-shell")
    {
        if raw_args.len() != 2 {
            eprintln!("错误: `qn init-shell` 后需要指定 fish、bash 或 zsh");
            return ExitCode::from(2);
        }
        return match install_shell_integration(&raw_args[1]) {
            Ok(message) => {
                println!("{message}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("错误: {error}");
                ExitCode::from(2)
            }
        };
    }
    if raw_args.len() == 1 && raw_args[0] == "__is-configured" {
        return if is_configured() {
            ExitCode::SUCCESS
        } else {
            ExitCode::from(1)
        };
    }
    if raw_args
        .first()
        .is_some_and(|argument| argument == "__notify")
    {
        return match notify_shell_report(raw_args.into_iter().skip(1)) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("警告: {error}");
                ExitCode::from(1)
            }
        };
    }

    let options = match parse_options() {
        Ok(Invocation::Run(options)) => options,
        Ok(Invocation::Text(content)) => {
            return match notify_text(&content) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("错误: {error}");
                    ExitCode::from(1)
                }
            };
        }
        Ok(Invocation::Markdown(content)) => {
            return match notify_markdown(&content) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("错误: {error}");
                    ExitCode::from(1)
                }
            };
        }
        Ok(Invocation::Media { path, media_type }) => {
            return match send_media(&path, media_type) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("错误: {error}");
                    ExitCode::from(1)
                }
            };
        }
        Ok(Invocation::Status) => {
            return match fetch_status() {
                Ok(status) => {
                    println!("connected={}\nbound={}", status.connected, status.bound);
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("错误: {error}");
                    ExitCode::from(1)
                }
            };
        }
        Err(error) => {
            eprintln!("错误: {error}");
            print_usage();
            return ExitCode::from(2);
        }
    };
    if !shell_integration_is_loaded() {
        warn_missing_shell_integration();
    }
    if options.notify && !is_configured() {
        eprintln!(
            "提示：qn 尚未初始化；运行 `qn init` 完成通知配置。本次命令会照常执行，但不会发送通知。"
        );
    }
    let started = Instant::now();
    let result = run(&options);
    let code = match &result {
        Ok(result) => result.code,
        Err(error) => {
            eprintln!("{error}");
            127
        }
    };
    let output = result
        .as_ref()
        .ok()
        .and_then(|result| result.output.as_ref())
        .map(command_output_display);
    if options.notify && is_configured() {
        if let Err(error) = notify_command_report(
            &command_display(&options),
            code,
            started.elapsed().as_secs(),
            output,
        ) {
            eprintln!("警告: {error}");
        }
    }
    ExitCode::from(code.clamp(0, u8::MAX as i32) as u8)
}

fn format_duration(seconds: u64) -> String {
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let seconds = seconds % 60;
    if hours > 0 {
        format!("{hours}h {minutes}m {seconds}s")
    } else if minutes > 0 {
        format!("{minutes}m {seconds}s")
    } else {
        format!("{seconds}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP_FILE_ID: AtomicU64 = AtomicU64::new(0);

    fn temporary_file_path(kind: &str) -> PathBuf {
        env::temp_dir().join(format!(
            "qn-{kind}-{}-{}",
            std::process::id(),
            NEXT_TEMP_FILE_ID.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn parses_attach_output_before_command() {
        let invocation =
            parse_options_from(["-a", "--no-notify", "echo", "hello"].map(String::from))
                .expect("options should parse");
        let Invocation::Run(options) = invocation else {
            panic!("expected command invocation");
        };

        assert!(options.attach_output);
        assert!(!options.notify);
        assert_eq!(options.command, ["echo", "hello"]);
    }

    #[test]
    fn parses_direct_text_and_markdown() {
        let text = parse_options_from(["--text", "部署完成"].map(String::from))
            .expect("text invocation should parse");
        let markdown = parse_options_from(["-m", "# 部署完成"].map(String::from))
            .expect("markdown invocation should parse");

        let Invocation::Text(content) = text else {
            panic!("expected text invocation");
        };
        assert_eq!(content, "部署完成");

        let Invocation::Markdown(content) = markdown else {
            panic!("expected markdown invocation");
        };
        assert_eq!(content, "# 部署完成");
    }

    #[test]
    fn parses_image_and_file_as_distinct_media_types() {
        let image = parse_options_from(["-i", "preview.png"].map(String::from))
            .expect("image invocation should parse");
        let file = parse_options_from(["--file", "report.pdf"].map(String::from))
            .expect("file invocation should parse");

        let Invocation::Media { path, media_type } = image else {
            panic!("expected image invocation");
        };
        assert_eq!(path, PathBuf::from("preview.png"));
        assert_eq!(media_type, MediaType::Image);

        let Invocation::Media { path, media_type } = file else {
            panic!("expected file invocation");
        };
        assert_eq!(path, PathBuf::from("report.pdf"));
        assert_eq!(media_type, MediaType::File);
    }

    #[test]
    fn rejects_direct_recipient_option() {
        let error =
            parse_options_from(["--text", "部署完成", "--to", "openid-1"].map(String::from))
                .expect_err("recipient targeting should be unavailable");
        assert_eq!(error, "当前服务只支持一个绑定接收人，不能指定 --to");
    }

    #[test]
    fn derives_native_routes_from_server_root() {
        assert_eq!(
            api_url_from_root("https://example.test", "v1/markdown"),
            "https://example.test/v1/markdown"
        );
        assert_eq!(
            api_url_from_root("https://example.test/", "status"),
            "https://example.test/status"
        );
    }

    #[test]
    fn parses_status_and_uses_explicit_media_api_values() {
        let invocation =
            parse_options_from(["--status"].map(String::from)).expect("status should parse");
        assert!(matches!(invocation, Invocation::Status));
        assert_eq!(MediaType::Image.api_value(), "image");
        assert_eq!(MediaType::File.api_value(), "file");
    }

    #[test]
    fn rejects_unknown_qn_option_before_command() {
        let error = parse_options_from(["-x", "echo"].map(String::from))
            .expect_err("unknown qn option should fail");

        assert_eq!(error, "未知 qn 选项：-x");
    }

    #[test]
    fn passes_hyphenated_arguments_after_command_to_command() {
        let invocation = parse_options_from(["echo", "--release", "-v"].map(String::from))
            .expect("command should parse");
        let Invocation::Run(options) = invocation else {
            panic!("expected command invocation");
        };

        assert_eq!(options.command, ["echo", "--release", "-v"]);
    }

    #[test]
    fn separator_allows_hyphenated_command_name() {
        let invocation = parse_options_from(["--", "-x", "argument"].map(String::from))
            .expect("command should parse");
        let Invocation::Run(options) = invocation else {
            panic!("expected command invocation");
        };

        assert_eq!(options.command, ["-x", "argument"]);
    }

    #[test]
    fn rejects_invalid_direct_message_arguments() {
        let empty =
            parse_options_from(["-t", ""].map(String::from)).expect_err("empty text should fail");
        assert_eq!(empty, "消息内容不能为空");

        let multiple = parse_options_from(["-m", "first", "second"].map(String::from))
            .expect_err("multiple markdown values should fail");
        assert_eq!(multiple, "只能指定一个Markdown 内容");

        let mixed = parse_options_from(["--no-notify", "-i", "preview.png"].map(String::from))
            .expect_err("direct invocation cannot use command options");
        assert_eq!(mixed, "直接发送选项不能与命令执行选项同时使用");
    }

    #[test]
    fn preserves_valid_utf8_output() {
        assert_eq!(
            decode_output_bytes("编译完成\n".as_bytes(), CapturedOutputEncoding::Auto),
            "编译完成\n"
        );
    }

    #[test]
    fn decodes_unicode_bom_and_explicit_utf16_output() {
        assert_eq!(
            decode_output_bytes(
                &[0xFF, 0xFE, 0x60, 0x4F, 0x7D, 0x59],
                CapturedOutputEncoding::Auto,
            ),
            "你好"
        );
        assert_eq!(
            decode_output_bytes(&[0x60, 0x4F, 0x7D, 0x59], CapturedOutputEncoding::Utf16Le,),
            "你好"
        );
        assert_eq!(
            decode_output_bytes(&[0x4F, 0x60, 0x59, 0x7D], CapturedOutputEncoding::Utf16Be,),
            "你好"
        );
    }

    #[test]
    fn detects_gbk_output_when_utf8_is_invalid() {
        let text = "你好，世界。编码检测应当恢复这段中文输出。";
        let (encoded, _, had_errors) = encoding_rs::GBK.encode(text);

        assert!(!had_errors);
        assert_eq!(
            decode_output_bytes(&encoded, CapturedOutputEncoding::Auto),
            text
        );
    }

    #[test]
    fn decodes_windows_console_code_page_output() {
        assert_eq!(
            decode_windows_code_page(&[0xC4, 0xE3, 0xBA, 0xC3], 936),
            Some("你好".into())
        );
    }

    #[test]
    #[cfg(unix)]
    fn attaches_standard_output_and_error_to_markdown_notification() {
        let options = Options {
            command: vec![
                "sh".into(),
                "-c".into(),
                "printf output; printf error >&2".into(),
            ],
            shell: None,
            notify: false,
            attach_output: true,
        };
        let result = run(&options).expect("command should run");

        assert_eq!(result.code, 0);
        let notification = build_markdown_notification(
            "MacBook-Pro",
            &command_display(&options),
            result.code,
            0,
            "/workspace/qn",
            result.output.as_ref().map(command_output_display),
        );
        assert!(notification.markdown.starts_with("## 任务完成"));
        assert!(notification.markdown.contains("**设备**：MacBook\\-Pro"));
        assert!(notification.markdown.contains("### 输出\n````text"));
        assert!(notification.markdown.contains("标准输出：\noutput"));
        assert!(notification.markdown.contains("标准错误：\nerror"));
        assert_eq!(notification.output_attachment, None);
    }

    #[test]
    fn truncates_long_output_and_keeps_full_log_as_attachment() {
        let output = "x".repeat(MAX_INLINE_OUTPUT_CHARS + 1);
        let notification = build_markdown_notification(
            "MacBook-Pro",
            "echo output",
            0,
            0,
            "/workspace/qn",
            Some(output.clone()),
        );

        assert!(notification.markdown.contains("完整日志作为附件发送"));
        assert!(notification.markdown.contains("完整输出见附件"));
        assert_eq!(notification.output_attachment, Some(output));
        assert!(notification.markdown.chars().count() <= MAX_MARKDOWN_CONTENT_CHARS);
    }

    #[test]
    fn notification_starts_with_device_name() {
        assert_eq!(
            notification_summary("MacBook-Pro", "任务完成\n命令：echo hello"),
            "MacBook-Pro\n任务完成\n命令：echo hello"
        );
    }

    #[test]
    fn replaces_localhost_with_macos_local_hostname() {
        assert_eq!(
            select_default_device_name("localhost".into(), Some("Krust-MacBook-Pro".into())),
            Ok("Krust-MacBook-Pro".into())
        );
    }

    #[test]
    fn preserves_non_localhost_hostname() {
        assert_eq!(
            select_default_device_name("build-server".into(), Some("MacBook-Pro".into())),
            Ok("build-server".into())
        );
    }

    #[test]
    fn uses_platform_native_config_directory() {
        let expected = BaseDirs::new()
            .expect("current platform should provide a configuration directory")
            .config_dir()
            .join("qn")
            .join("config");

        assert_eq!(config_path(), Ok(expected));
    }

    #[test]
    fn initializes_config_from_standard_io() {
        let path = temporary_file_path("init-test");
        let mut input = io::Cursor::new(b"\ntoken-value\n\n");
        let mut output = Vec::new();

        initialize_config_with_io(&path, None, &mut input, &mut output, || {
            Ok("test-host".into())
        })
        .expect("configuration should initialize");

        assert_eq!(
            fs::read_to_string(&path).expect("configuration should be readable"),
            format!("endpoint={DEFAULT_ENDPOINT}\ntoken=token-value\nname=test-host\n")
        );
        assert!(
            String::from_utf8(output)
                .expect("prompts should be UTF-8")
                .contains("配置已保存")
        );
        fs::remove_file(path).expect("temporary config should be removed");
    }

    #[test]
    fn missing_device_name_uses_hostname_and_persists_it() {
        let path = temporary_file_path("config-test");
        let expected = default_device_name().expect("hostname should be available");

        let name = device_name_from_config(&path).expect("device name should be saved");

        assert_eq!(name, expected);
        assert_eq!(
            read_config_value_from(&path, "name"),
            Some(expected),
            "configuration contents: {:?}",
            fs::read_to_string(&path)
        );
        fs::remove_file(path).expect("temporary config should be removed");
    }

    #[test]
    fn configured_device_name_is_used() {
        let path = temporary_file_path("config-test");
        fs::write(&path, "name=办公室 Mac\n").expect("temporary config should be written");

        let name = device_name_from_config(&path).expect("configured name should be read");

        assert_eq!(name, "办公室 Mac");
        fs::remove_file(path).expect("temporary config should be removed");
    }
    #[test]
    fn parses_shell_report_with_captured_output() {
        let report = parse_shell_report(
            [
                "--command",
                "deploy preview",
                "--exit-code",
                "17",
                "--elapsed",
                "42",
                "--stdout-file",
                "/tmp/qn-stdout",
                "--stderr-file",
                "/tmp/qn-stderr",
            ]
            .map(String::from),
        )
        .expect("shell report should parse");

        assert_eq!(
            report,
            ShellReport {
                command: "deploy preview".into(),
                code: 17,
                elapsed_seconds: 42,
                stdout_file: Some(PathBuf::from("/tmp/qn-stdout")),
                stderr_file: Some(PathBuf::from("/tmp/qn-stderr")),
                output_encoding: CapturedOutputEncoding::Auto,
            }
        );
    }

    #[test]
    fn parses_explicit_shell_report_output_encoding() {
        let report = parse_shell_report(
            [
                "--command",
                "Write-Output 你好",
                "--exit-code",
                "0",
                "--elapsed",
                "1",
                "--stdout-file",
                "/tmp/qn-stdout",
                "--stderr-file",
                "/tmp/qn-stderr",
                "--output-encoding",
                "utf-16le",
            ]
            .map(String::from),
        )
        .expect("shell report should parse");

        assert_eq!(report.output_encoding, CapturedOutputEncoding::Utf16Le);
    }

    #[test]
    fn rejects_shell_report_with_only_one_output_file() {
        let error = parse_shell_report(
            [
                "--command",
                "echo hello",
                "--exit-code",
                "0",
                "--elapsed",
                "1",
                "--stdout-file",
                "/tmp/qn-stdout",
            ]
            .map(String::from),
        )
        .expect_err("unpaired output files should fail");

        assert!(error.contains("必须同时提供"));
    }

    #[test]
    fn treats_missing_captured_output_as_empty() {
        let path = temporary_file_path("missing-output");

        let output =
            read_captured_output_file(&path, "标准输出").expect("missing output should be empty");

        assert!(output.is_empty());
    }

    #[test]
    fn writes_shell_integration_once_and_migrates_legacy_loader() {
        let path = env::temp_dir().join(format!(
            "qn-shell-integration-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time should be after Unix epoch")
                .as_nanos()
        ));
        fs::write(
            &path,
            format!(
                "export PATH\n{}\n",
                ShellKind::Bash.legacy_integration_command()
            ),
        )
        .expect("legacy loader should be written");

        assert!(
            write_shell_integration(&path, ShellKind::Bash).expect("integration should be written")
        );
        assert!(
            !write_shell_integration(&path, ShellKind::Bash)
                .expect("integration should not change")
        );
        let contents = fs::read_to_string(&path).expect("integration should be readable");
        assert!(contents.starts_with("export PATH\n"));
        assert!(contents.contains(SHELL_INTEGRATION_BEGIN_MARKER));
        assert!(contents.contains(BASH_SHELL_INIT));
        assert!(contents.contains(SHELL_INTEGRATION_END_MARKER));
        assert!(!contents.contains(ShellKind::Bash.legacy_integration_command()));

        fs::remove_file(path).expect("temporary config should be removed");
    }

    #[cfg(windows)]
    #[test]
    fn powershell_shell_initialization_parses() {
        let path = temporary_file_path("powershell-profile");
        assert!(
            write_shell_integration(&path, ShellKind::PowerShell)
                .expect("PowerShell integration should be written")
        );
        let profile = fs::read(&path).expect("PowerShell profile should be readable");
        assert!(profile.starts_with(&[0xEF, 0xBB, 0xBF]));
        assert!(
            !write_shell_integration(&path, ShellKind::PowerShell)
                .expect("PowerShell integration should already use UTF-8 BOM")
        );

        let escaped_path = path.to_string_lossy().replace('\'', "''");
        let command = format!("& {{ . '{escaped_path}' }}");
        let output = Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                command.as_str(),
            ])
            .output()
            .expect("PowerShell should be available on Windows");

        assert!(
            output.status.success(),
            "PowerShell parser failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        fs::remove_file(path).expect("temporary PowerShell profile should be removed");
    }

    #[test]
    fn provides_shell_integrations_for_supported_shells() {
        for shell in [
            ShellKind::Fish,
            ShellKind::Bash,
            ShellKind::Zsh,
            ShellKind::PowerShell,
        ] {
            let integration = shell.init_script();
            assert!(integration.contains("function qn") || integration.contains("qn()"));
            assert!(integration.contains("__notify"));
            assert!(integration.contains(SHELL_INTEGRATION_ENV));
            for option in ["--text", "--markdown", "--image", "--file", "--status"] {
                assert!(
                    integration.contains(option),
                    "{shell:?} integration should delegate {option}"
                );
            }
        }
        assert!(FISH_SHELL_INIT.contains("touch \"$qn_stdout\" \"$qn_stderr\""));
        assert!(POWERSHELL_SHELL_INIT.contains("Get-Command qn -CommandType Application"));
        assert!(POWERSHELL_SHELL_INIT.contains("Invoke-Expression"));
        assert!(POWERSHELL_SHELL_INIT.contains("$qn_args.ToArray() -join \" \""));
        assert!(POWERSHELL_SHELL_INIT.contains("--output-encoding"));
        assert!(POWERSHELL_SHELL_INIT.contains("PSVersionTable.PSVersion.Major -lt 6"));
        assert!(!POWERSHELL_SHELL_INIT.contains("[string[]]$qn_args.ToArray()"));
        assert_eq!(
            ShellKind::from_name("powershell"),
            Some(ShellKind::PowerShell)
        );
        assert!(ShellKind::from_name("sh").is_none());
    }
}
