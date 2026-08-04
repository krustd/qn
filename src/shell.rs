use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) const SHELL_INTEGRATION_ENV: &str = "QN_SHELL_INTEGRATION";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ShellKind {
    Fish,
    Bash,
    Zsh,
    PowerShell,
}

impl ShellKind {
    pub(crate) fn from_process_command(command: &str) -> Option<Self> {
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

    pub(crate) fn from_name(name: &str) -> Option<Self> {
        match name {
            "fish" => Some(Self::Fish),
            "bash" => Some(Self::Bash),
            "zsh" => Some(Self::Zsh),
            "powershell" => Some(Self::PowerShell),
            _ => None,
        }
    }

    pub(crate) fn display_name(self) -> &'static str {
        match self {
            Self::Fish => "Fish",
            Self::Bash => "Bash",
            Self::Zsh => "Zsh",
            Self::PowerShell => "PowerShell",
        }
    }

    pub(crate) fn legacy_integration_command(self) -> &'static str {
        match self {
            Self::Fish => "qn shell-init fish | source",
            Self::Bash => "eval \"$(qn shell-init bash)\"",
            Self::Zsh => "eval \"$(qn shell-init zsh)\"",
            Self::PowerShell => "qn shell-init powershell | Invoke-Expression",
        }
    }

    pub(crate) fn init_script(self) -> &'static str {
        match self {
            Self::Fish => FISH_SHELL_INIT,
            Self::Bash => BASH_SHELL_INIT,
            Self::Zsh => ZSH_SHELL_INIT,
            Self::PowerShell => POWERSHELL_SHELL_INIT,
        }
    }

    pub(crate) fn startup_config_path(self) -> Result<PathBuf, String> {
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

    pub(crate) fn install_command(self) -> &'static str {
        match self {
            Self::Fish => "qn init-shell fish",
            Self::Bash => "qn init-shell bash",
            Self::Zsh => "qn init-shell zsh",
            Self::PowerShell => "qn init-shell powershell",
        }
    }
}

#[cfg(windows)]
pub(crate) fn powershell_profile_path() -> Result<PathBuf, String> {
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
pub(crate) fn powershell_profile_path() -> Result<PathBuf, String> {
    Err("PowerShell 集成仅支持 Windows".into())
}

pub(crate) const FISH_SHELL_INIT: &str = r#"function qn
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

pub(crate) const BASH_SHELL_INIT: &str = r#"qn() {
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

pub(crate) const POWERSHELL_SHELL_INIT: &str = r#"function qn {
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
pub(crate) fn shell_integration_is_loaded() -> bool {
    env::var(SHELL_INTEGRATION_ENV).is_ok_and(|value| value == "1")
}

#[cfg(unix)]
pub(crate) fn process_info(pid: u32) -> Option<(u32, String)> {
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
pub(crate) fn shell_from_ancestor_processes() -> Option<ShellKind> {
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
pub(crate) fn shell_from_ancestor_processes() -> Option<ShellKind> {
    env::var_os("PSModulePath").map(|_| ShellKind::PowerShell)
}

#[cfg(all(not(unix), not(windows)))]
pub(crate) fn shell_from_ancestor_processes() -> Option<ShellKind> {
    None
}

pub(crate) fn warn_missing_shell_integration() {
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

pub(crate) const SHELL_INTEGRATION_BEGIN_MARKER: &str = "# >>> qn shell integration >>>";
pub(crate) const SHELL_INTEGRATION_END_MARKER: &str = "# <<< qn shell integration <<<";

pub(crate) fn shell_integration_uses_utf8_bom(shell: ShellKind) -> bool {
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

pub(crate) fn write_shell_integration_contents(
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

pub(crate) fn write_shell_integration(path: &Path, shell: ShellKind) -> Result<bool, String> {
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

pub(crate) fn install_shell_integration(shell_name: &str) -> Result<String, String> {
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
