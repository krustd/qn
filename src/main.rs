use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::process::{Command, ExitCode};
use std::time::Instant;

use reqwest::blocking::Client;
use serde::Serialize;

const DEFAULT_ENDPOINT: &str = "https://krust.iepose.cn/task-completed";
#[derive(Debug)]
struct Options {
    command: Vec<String>,
    shell: Option<String>,
    notify: bool,
}

#[derive(Serialize)]
struct Notification<'a> {
    summary: &'a str,
}

fn config_path() -> Result<PathBuf, String> {
    let home = env::var_os("HOME").ok_or("无法确定用户目录，请设置 HOME")?;
    Ok(PathBuf::from(home).join(".config/qn/config"))
}

fn read_config_value(name: &str) -> Option<String> {
    let path = config_path().ok()?;
    let contents = fs::read_to_string(path).ok()?;
    contents.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        (key.trim() == name).then(|| value.trim().to_owned())
    })
}

fn initialize_config() -> Result<(), String> {
    let path = config_path()?;
    let tty = fs::File::options()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .map_err(|_| "配置需要交互式终端，请设置 QN_ENDPOINT 和 QN_TOKEN 环境变量后重试".to_owned())?;
    let mut input = io::BufReader::new(tty.try_clone().map_err(|error| error.to_string())?);
    let mut output = tty;
    let endpoint = match env::var("QN_ENDPOINT") {
        Ok(e) => e,
        Err(_) => {
            writeln!(
                output,
                "请输入 QN_ENDPOINT（默认 {DEFAULT_ENDPOINT}）："
            )
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
    writeln!(
        output,
        "请输入 QN_TOKEN："
    )
    .map_err(|error| error.to_string())?;
    output.flush().map_err(|error| error.to_string())?;
    let mut token = String::new();
    input
        .read_line(&mut token)
        .map_err(|error| error.to_string())?;
    let token = token.trim();
    if token.is_empty() {
        return Err("QN_TOKEN 不能为空".into());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::write(&path, format!("endpoint={endpoint}\ntoken={token}\n"))
        .map_err(|error| format!("写入配置失败: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())?;
    }
    println!("配置已保存：{}", path.display());
    Ok(())
}

fn print_usage() {
    eprintln!("用法:");
    eprintln!("  qn [--no-notify] <command> [args...]");
    eprintln!("  qn [--no-notify] --shell <command-string>");
    eprintln!("  qn init");
    eprintln!();
    eprintln!("环境变量:");
    eprintln!("  QN_ENDPOINT  通知接口 URL（`qn init` 时留空则默认 {DEFAULT_ENDPOINT}）");
    eprintln!("  QN_TOKEN     通知接口 Bearer Token");
}

fn parse_options() -> Result<Options, String> {
    let mut args = env::args().skip(1).peekable();
    let mut notify = true;
    let mut shell = None;
    let mut command = Vec::new();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" if command.is_empty() && shell.is_none() => {
                print_usage();
                std::process::exit(0);
            }
            "--no-notify" if command.is_empty() && shell.is_none() => notify = false,
            "--shell" if command.is_empty() && shell.is_none() => {
                shell = Some(args.next().ok_or("--shell 后需要命令字符串")?);
            }
            "--" if command.is_empty() && shell.is_none() => {
                command.extend(args);
                break;
            }
            _ => command.push(arg),
        }
    }

    if shell.is_none() && command.is_empty() {
        return Err("没有指定要执行的命令".into());
    }
    Ok(Options {
        command,
        shell,
        notify,
    })
}

fn run(options: &Options) -> Result<i32, String> {
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

    let status = process
        .status()
        .map_err(|error| format!("启动命令失败: {error}"))?;
    Ok(status.code().unwrap_or(1))
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

fn notify(summary: &str) -> Result<(), String> {
    let token = env::var("QN_TOKEN")
        .ok()
        .or_else(|| read_config_value("token"))
        .ok_or("未配置 QN_TOKEN，请运行 `qn init` 或设置环境变量".to_owned())?;
    let endpoint = env::var("QN_ENDPOINT")
        .ok()
        .or_else(|| read_config_value("endpoint"))
        .ok_or("未配置 QN_ENDPOINT，请运行 `qn init` 或设置环境变量".to_owned())?;
    Client::new()
        .post(endpoint)
        .bearer_auth(token)
        .json(&Notification { summary })
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
    let options = match parse_options() {
        Ok(options) => options,
        Err(error) => {
            eprintln!("错误: {error}");
            print_usage();
            return ExitCode::from(2);
        }
    };
    let started = Instant::now();
    let result = run(&options);
    let code = match result {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{error}");
            127
        }
    };
    let state = if code == 0 {
        "任务完成"
    } else {
        "任务失败"
    };
    let summary = format!(
        "{state}\n命令：{}\n退出码：{code}\n耗时：{}\n工作目录：{}",
        command_display(&options),
        format_duration(started.elapsed().as_secs()),
        env::current_dir().map_or_else(|_| "未知".into(), |path| path.display().to_string())
    );
    if options.notify {
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
