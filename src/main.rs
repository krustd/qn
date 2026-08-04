use std::env;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

#[cfg(test)]
use std::fs;
#[cfg(test)]
use std::io;
#[cfg(all(test, windows))]
use std::process::Command;

#[cfg(test)]
use directories::BaseDirs;

use serde::{Deserialize, Serialize};

mod cli;
mod config;
mod notification;
mod shell;

use cli::*;
use config::*;
use notification::*;
use shell::*;

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
