use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::Instant;

use reqwest::blocking::Client;
use serde::Serialize;

const DEFAULT_ENDPOINT: &str = "https://krust.iepose.cn/task-completed";
const SHELL_INTEGRATION_ENV: &str = "QN_SHELL_INTEGRATION";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShellKind {
    Fish,
    Bash,
    Zsh,
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
            _ => None,
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        match name {
            "fish" => Some(Self::Fish),
            "bash" => Some(Self::Bash),
            "zsh" => Some(Self::Zsh),
            _ => None,
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::Fish => "Fish",
            Self::Bash => "Bash",
            Self::Zsh => "Zsh",
        }
    }

    fn legacy_integration_command(self) -> &'static str {
        match self {
            Self::Fish => "qn shell-init fish | source",
            Self::Bash => "eval \"$(qn shell-init bash)\"",
            Self::Zsh => "eval \"$(qn shell-init zsh)\"",
        }
    }

    fn init_script(self) -> &'static str {
        match self {
            Self::Fish => FISH_SHELL_INIT,
            Self::Bash => BASH_SHELL_INIT,
            Self::Zsh => ZSH_SHELL_INIT,
        }
    }

    fn startup_config_path(self, home: &Path) -> PathBuf {
        match self {
            Self::Fish => home.join(".config/fish/config.fish"),
            Self::Bash => home.join(".bashrc"),
            Self::Zsh => home.join(".zshrc"),
        }
    }

    fn install_command(self) -> &'static str {
        match self {
            Self::Fish => "qn init-shell fish",
            Self::Bash => "qn init-shell bash",
            Self::Zsh => "qn init-shell zsh",
        }
    }
}

const FISH_SHELL_INIT: &str = r#"function qn
    set -lx QN_SHELL_INTEGRATION 1

    if test (count $argv) -gt 0
        switch $argv[1]
            case -h --help -m --message init init-shell __notify __is-configured
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
            case -m --message
                command qn $qn_original
                return $status
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
        -h|--help|-m|--message|init|init-shell|__notify|__is-configured)
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
            -m|--message)
                command qn "${qn_original[@]}"
                return $?
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
        -h|--help|-m|--message|init|init-shell|__notify|__is-configured)
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
            -m|--message)
                command qn "${qn_original[@]}"
                return $?
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
#[derive(Debug)]
enum Invocation {
    Run(Options),
    Message(String),
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
}

#[derive(Serialize)]
struct Notification<'a> {
    summary: &'a str,
}

fn config_path() -> Result<PathBuf, String> {
    let home = env::var_os("HOME").ok_or("无法确定用户目录，请设置 HOME")?;
    Ok(PathBuf::from(home).join(".config/qn/config"))
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

#[cfg(not(unix))]
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

fn write_shell_integration(path: &Path, shell: ShellKind) -> Result<bool, String> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(format!(
                "读取 Shell 启动配置失败（{}）: {error}",
                path.display()
            ));
        }
    };
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

    if updated == contents {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("创建 Shell 配置目录失败（{}）: {error}", parent.display()))?;
    }
    let is_new = !path.exists();
    fs::write(path, updated)
        .map_err(|error| format!("写入 Shell 启动配置失败（{}）: {error}", path.display()))?;
    #[cfg(unix)]
    if is_new {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("设置配置权限失败: {error}"))?;
    }
    Ok(true)
}

fn install_shell_integration(shell_name: &str) -> Result<String, String> {
    let shell = ShellKind::from_name(shell_name)
        .ok_or_else(|| format!("不支持的 Shell：{shell_name}；请使用 fish、bash 或 zsh"))?;
    let home = env::var_os("HOME").ok_or("无法确定用户目录，请设置 HOME")?;
    let path = shell.startup_config_path(&PathBuf::from(home));
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

fn default_device_name() -> Result<String, String> {
    let name = hostname::get()
        .map_err(|error| format!("获取主机名失败: {error}"))?
        .to_string_lossy()
        .trim()
        .to_owned();
    if name.is_empty() {
        Err("主机名为空，无法设置设备名称".into())
    } else {
        Ok(name)
    }
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

fn initialize_config() -> Result<(), String> {
    let path = config_path()?;
    let tty = fs::File::options()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .map_err(|_| {
            "配置需要交互式终端，请设置 QN_ENDPOINT 和 QN_TOKEN 环境变量后重试".to_owned()
        })?;
    let mut input = io::BufReader::new(tty.try_clone().map_err(|error| error.to_string())?);
    let mut output = tty;
    let endpoint = match env::var("QN_ENDPOINT") {
        Ok(e) => e,
        Err(_) => {
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
    let default_name = default_device_name()?;
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
    fs::write(
        &path,
        format!("endpoint={endpoint}\ntoken={token}\nname={name}\n"),
    )
    .map_err(|error| format!("写入配置失败: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())?;
    }
    println!("配置已保存：{}", path.display());
    println!("要让 qn 使用当前 Shell 的 alias 和 function，请运行对应命令写入启动配置：");
    println!("  fish: qn init-shell fish");
    println!("  bash: qn init-shell bash");
    println!("  zsh:  qn init-shell zsh");
    Ok(())
}

fn print_usage() {
    eprintln!("用法:");
    eprintln!("  qn [-a|--attach-output] [--no-notify] <command> [args...]");
    eprintln!("  qn [-a|--attach-output] [--no-notify] --shell <command-string>");
    eprintln!("  qn -m|--message <message>");
    eprintln!("  qn init");
    eprintln!("  qn init-shell <fish|bash|zsh>");
    eprintln!();
    eprintln!("选项:");
    eprintln!("  -a, --attach-output  在通知中附带命令的标准输出和标准错误");
    eprintln!("  --no-notify          不发送完成通知");
    eprintln!("  -m, --message        直接发送消息，不执行命令");
    eprintln!();
    eprintln!("环境变量:");
    eprintln!("  QN_ENDPOINT  通知接口 URL（`qn init` 时留空则默认 {DEFAULT_ENDPOINT}）");
    eprintln!("  QN_TOKEN     通知接口 Bearer Token");
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
            "-m" | "--message" => {
                if !notify || attach_output || shell.is_some() {
                    return Err("-m/--message 不能与命令执行选项同时使用".into());
                }
                let message = args.next().ok_or("-m/--message 后需要消息内容")?;
                if message.is_empty() {
                    return Err("消息不能为空".into());
                }
                if args.next().is_some() {
                    return Err("-m/--message 后不能再指定命令或其他参数".into());
                }
                return Ok(Invocation::Message(message));
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
            _ => return Err(format!("未知或重复的通知参数：{argument}")),
        }
    }

    let (stdout_file, stderr_file) = match (stdout_file, stderr_file) {
        (Some(stdout_file), Some(stderr_file)) => (Some(stdout_file), Some(stderr_file)),
        (None, None) => (None, None),
        _ => return Err("标准输出和标准错误文件必须同时提供".into()),
    };
    Ok(ShellReport {
        command: command.ok_or("--command 不能为空")?,
        code: code.ok_or("--exit-code 不能为空")?,
        elapsed_seconds: elapsed_seconds.ok_or("--elapsed 不能为空")?,
        stdout_file,
        stderr_file,
    })
}

fn notify_shell_report(args: impl IntoIterator<Item = String>) -> Result<(), String> {
    let report = parse_shell_report(args)?;
    if !is_configured() {
        return Ok(());
    }
    let output = match (report.stdout_file, report.stderr_file) {
        (Some(stdout_file), Some(stderr_file)) => {
            let stdout = fs::read(&stdout_file).map_err(|error| {
                format!("读取标准输出失败（{}）: {error}", stdout_file.display())
            })?;
            let stderr = fs::read(&stderr_file).map_err(|error| {
                format!("读取标准错误失败（{}）: {error}", stderr_file.display())
            })?;
            Some(command_output_display_bytes(&stdout, &stderr))
        }
        (None, None) => None,
        _ => unreachable!("parse_shell_report ensures output files are paired"),
    };
    notify(&build_summary_for_command(
        &report.command,
        report.code,
        report.elapsed_seconds,
        output.as_deref(),
    ))
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
    command_output_display_bytes(&output.stdout, &output.stderr)
}

fn command_output_display_bytes(stdout: &[u8], stderr: &[u8]) -> String {
    let stdout = String::from_utf8_lossy(stdout);
    let stderr = String::from_utf8_lossy(stderr);
    match (stdout.is_empty(), stderr.is_empty()) {
        (true, true) => "（无输出）".to_owned(),
        (false, true) => format!("标准输出：\n{stdout}"),
        (true, false) => format!("标准错误：\n{stderr}"),
        (false, false) => format!("标准输出：\n{stdout}\n标准错误：\n{stderr}"),
    }
}

fn build_summary(
    options: &Options,
    code: i32,
    elapsed_seconds: u64,
    output: Option<&std::process::Output>,
) -> String {
    let output = output.map(command_output_display);
    build_summary_for_command(
        &command_display(options),
        code,
        elapsed_seconds,
        output.as_deref(),
    )
}

fn build_summary_for_command(
    command: &str,
    code: i32,
    elapsed_seconds: u64,
    output: Option<&str>,
) -> String {
    let state = if code == 0 {
        "任务完成"
    } else {
        "任务失败"
    };
    let mut summary = format!(
        "{state}\n命令：{command}\n退出码：{code}\n耗时：{}\n工作目录：{}",
        format_duration(elapsed_seconds),
        env::current_dir().map_or_else(|_| "未知".into(), |path| path.display().to_string())
    );
    if let Some(output) = output {
        summary.push_str("\n输出：\n");
        summary.push_str(output);
    }
    summary
}

fn notification_summary(device_name: &str, summary: &str) -> String {
    format!("{device_name}\n{summary}")
}

fn notify(summary: &str) -> Result<(), String> {
    let token = configured_value("QN_TOKEN", "token")
        .ok_or("未配置 QN_TOKEN，请运行 `qn init` 或设置环境变量".to_owned())?;
    let endpoint = configured_value("QN_ENDPOINT", "endpoint")
        .ok_or("未配置 QN_ENDPOINT，请运行 `qn init` 或设置环境变量".to_owned())?;
    let summary = notification_summary(&device_name()?, summary);
    Client::new()
        .post(endpoint)
        .bearer_auth(token)
        .json(&Notification { summary: &summary })
        .send()
        .map_err(|error| format!("发送通知失败: {error}"))?
        .error_for_status()
        .map(|_| ())
        .map_err(|error| format!("通知接口返回错误: {error}"))
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
        Ok(Invocation::Message(message)) => {
            return match notify(&message) {
                Ok(()) => ExitCode::SUCCESS,
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
    let summary = build_summary(
        &options,
        code,
        started.elapsed().as_secs(),
        result
            .as_ref()
            .ok()
            .and_then(|result| result.output.as_ref()),
    );
    if options.notify && is_configured() {
        if let Err(error) = notify(&summary) {
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
    fn parses_direct_message() {
        let invocation = parse_options_from(["--message", "部署完成"].map(String::from))
            .expect("message should parse");
        let Invocation::Message(message) = invocation else {
            panic!("expected message invocation");
        };

        assert_eq!(message, "部署完成");
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
    fn rejects_empty_direct_message() {
        let error = parse_options_from(["-m", ""].map(String::from))
            .expect_err("empty message should fail");

        assert_eq!(error, "消息不能为空");
    }

    #[test]
    #[cfg(unix)]
    fn attaches_standard_output_and_error_to_summary() {
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
        let summary = build_summary(&options, result.code, 0, result.output.as_ref());
        assert!(summary.contains("标准输出：\noutput"));
        assert!(summary.contains("标准错误：\nerror"));
    }

    #[test]
    fn notification_starts_with_device_name() {
        assert_eq!(
            notification_summary("MacBook-Pro", "任务完成\n命令：echo hello"),
            "MacBook-Pro\n任务完成\n命令：echo hello"
        );
    }

    #[test]
    fn missing_device_name_uses_hostname_and_persists_it() {
        let path = env::temp_dir().join(format!(
            "qn-config-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time should be after Unix epoch")
                .as_nanos()
        ));
        let expected = default_device_name().expect("hostname should be available");

        let name = device_name_from_config(&path).expect("device name should be saved");

        assert_eq!(name, expected);
        assert_eq!(read_config_value_from(&path, "name"), Some(expected));
        fs::remove_file(path).expect("temporary config should be removed");
    }

    #[test]
    fn configured_device_name_is_used() {
        let path = env::temp_dir().join(format!(
            "qn-config-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time should be after Unix epoch")
                .as_nanos()
        ));
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
            }
        );
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

    #[test]
    fn provides_shell_integrations_for_supported_shells() {
        for shell in [ShellKind::Fish, ShellKind::Bash, ShellKind::Zsh] {
            let integration = shell.init_script();
            assert!(integration.contains("function qn") || integration.contains("qn()"));
            assert!(integration.contains("__notify"));
            assert!(integration.contains(SHELL_INTEGRATION_ENV));
            assert!(
                integration.contains("--message"),
                "{shell:?} integration should delegate direct messages"
            );
        }
        assert!(ShellKind::from_name("sh").is_none());
    }
}
