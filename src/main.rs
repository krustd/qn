use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
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
    attach_output: bool,
}

#[derive(Debug)]
struct CommandResult {
    code: i32,
    output: Option<std::process::Output>,
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
    Ok(())
}

fn print_usage() {
    eprintln!("用法:");
    eprintln!("  qn [-a|--attach-output] [--no-notify] <command> [args...]");
    eprintln!("  qn [-a|--attach-output] [--no-notify] --shell <command-string>");
    eprintln!("  qn init");
    eprintln!();
    eprintln!("选项:");
    eprintln!("  -a, --attach-output  在通知中附带命令的标准输出和标准错误");
    eprintln!("  --no-notify          不发送完成通知");
    eprintln!();
    eprintln!("环境变量:");
    eprintln!("  QN_ENDPOINT  通知接口 URL（`qn init` 时留空则默认 {DEFAULT_ENDPOINT}）");
    eprintln!("  QN_TOKEN     通知接口 Bearer Token");
}

fn parse_options() -> Result<Options, String> {
    parse_options_from(env::args().skip(1))
}

fn parse_options_from(args: impl IntoIterator<Item = String>) -> Result<Options, String> {
    let mut args = args.into_iter().peekable();
    let mut notify = true;
    let mut attach_output = false;
    let mut shell = None;
    let mut command = Vec::new();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" if command.is_empty() && shell.is_none() => {
                print_usage();
                std::process::exit(0);
            }
            "--no-notify" if command.is_empty() && shell.is_none() => notify = false,
            "-a" | "--attach-output" if command.is_empty() && shell.is_none() => {
                attach_output = true
            }
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
        attach_output,
    })
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
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
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
    let state = if code == 0 {
        "任务完成"
    } else {
        "任务失败"
    };
    let mut summary = format!(
        "{state}\n命令：{}\n退出码：{code}\n耗时：{}\n工作目录：{}",
        command_display(options),
        format_duration(elapsed_seconds),
        env::current_dir().map_or_else(|_| "未知".into(), |path| path.display().to_string())
    );
    if let Some(output) = output {
        summary.push_str("\n输出：\n");
        summary.push_str(&command_output_display(output));
    }
    summary
}

fn notification_summary(device_name: &str, summary: &str) -> String {
    format!("{device_name}\n{summary}")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_attach_output_before_command() {
        let options = parse_options_from(["-a", "--no-notify", "echo", "hello"].map(String::from))
            .expect("options should parse");

        assert!(options.attach_output);
        assert!(!options.notify);
        assert_eq!(options.command, ["echo", "hello"]);
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
}
