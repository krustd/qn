use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use chardetng::EncodingDetector;
use encoding_rs::{Encoding, UTF_16BE, UTF_16LE};
use reqwest::blocking::Client;

use crate::config::{api_url, configured_value, device_name, is_configured};
use crate::{
    ApiMessage, CapturedOutputEncoding, CommandNotification, CommandResult, MediaType,
    Notification, Options, ShellReport, StatusResponse,
};
pub(crate) fn parse_shell_report(
    args: impl IntoIterator<Item = String>,
) -> Result<ShellReport, String> {
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

pub(crate) fn read_captured_output_file(path: &Path, output_name: &str) -> Result<Vec<u8>, String> {
    match fs::read(path) {
        Ok(output) => Ok(output),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(format!(
            "读取{output_name}失败（{}）: {error}",
            path.display()
        )),
    }
}

pub(crate) fn notify_shell_report(args: impl IntoIterator<Item = String>) -> Result<(), String> {
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

pub(crate) fn run(options: &Options) -> Result<CommandResult, String> {
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

pub(crate) fn command_display(options: &Options) -> String {
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

pub(crate) fn shell_quote(value: &str) -> String {
    if value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "_./:@%+=,-".contains(character))
    {
        value.to_owned()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

pub(crate) fn command_output_display(output: &std::process::Output) -> String {
    command_output_display_bytes(&output.stdout, &output.stderr, CapturedOutputEncoding::Auto)
}

pub(crate) fn decode_with_encoding(bytes: &[u8], encoding: &'static Encoding) -> String {
    let (text, _, _) = encoding.decode(bytes);
    text.into_owned()
}

#[cfg(any(windows, test))]
pub(crate) fn decode_windows_code_page(bytes: &[u8], code_page: u16) -> Option<String> {
    let encoding = codepage::to_encoding(code_page)?;
    let (text, _, had_errors) = encoding.decode(bytes);
    (!had_errors).then(|| text.into_owned())
}

#[cfg(windows)]
pub(crate) fn windows_console_output_code_page() -> Option<u16> {
    use windows_sys::Win32::System::Console::GetConsoleOutputCP;

    u16::try_from(unsafe { GetConsoleOutputCP() }).ok()
}

pub(crate) fn decode_output_bytes(bytes: &[u8], output_encoding: CapturedOutputEncoding) -> String {
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

pub(crate) fn command_output_display_bytes(
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
pub(crate) const MAX_INLINE_OUTPUT_CHARS: usize = 1_000;
pub(crate) const MAX_MARKDOWN_CONTENT_CHARS: usize = 4_000;

pub(crate) fn truncate_text(
    value: &str,
    maximum_characters: usize,
    suffix: &str,
) -> (String, bool) {
    if value.chars().count() <= maximum_characters {
        return (value.to_owned(), false);
    }
    let prefix_length = maximum_characters.saturating_sub(suffix.chars().count());
    let prefix: String = value.chars().take(prefix_length).collect();
    (format!("{prefix}{suffix}"), true)
}

pub(crate) fn markdown_text(value: &str, maximum_characters: usize) -> String {
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

pub(crate) fn markdown_code_block(language: &str, content: &str) -> String {
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

pub(crate) fn build_markdown_notification(
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

pub(crate) fn notify_command_report(
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

pub(crate) fn notification_summary(device_name: &str, summary: &str) -> String {
    format!("{device_name}\n{summary}")
}

pub(crate) fn api_token() -> Result<String, String> {
    configured_value("QN_TOKEN", "token")
        .ok_or("未配置 QN_TOKEN，请运行 `qn init` 或设置环境变量".to_owned())
}

pub(crate) fn notification_error(action: &str, error: reqwest::Error) -> String {
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

pub(crate) fn notify(summary: &str) -> Result<(), String> {
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

pub(crate) fn notify_text(content: &str) -> Result<(), String> {
    notify(content)
}

pub(crate) fn notify_markdown(content: &str) -> Result<(), String> {
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

pub(crate) fn send_media(path: &Path, media_type: MediaType) -> Result<(), String> {
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

pub(crate) fn send_output_attachment(output: String) -> Result<(), String> {
    let part = reqwest::blocking::multipart::Part::bytes(output.into_bytes())
        .file_name("qn-output.txt".to_owned());
    send_media_part(part, MediaType::File)
}

pub(crate) fn send_media_part(
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

pub(crate) fn fetch_status() -> Result<StatusResponse, String> {
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

pub(crate) fn format_duration(seconds: u64) -> String {
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
