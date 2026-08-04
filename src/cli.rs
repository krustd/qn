use std::env;
use std::path::PathBuf;

use crate::{Invocation, MediaType, Options};

pub(crate) fn print_usage() {
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

pub(crate) fn direct_options_are_allowed(
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

pub(crate) fn parse_direct_arguments(
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

pub(crate) fn parse_options() -> Result<Invocation, String> {
    parse_options_from(env::args().skip(1))
}

pub(crate) fn parse_options_from(
    args: impl IntoIterator<Item = String>,
) -> Result<Invocation, String> {
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
