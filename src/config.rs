use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use directories::BaseDirs;

pub(crate) const DEFAULT_ENDPOINT: &str = "https://krust.iepose.cn";
pub(crate) fn config_path() -> Result<PathBuf, String> {
    let base_dirs = BaseDirs::new().ok_or("无法确定当前用户的配置目录")?;
    Ok(base_dirs.config_dir().join("qn").join("config"))
}

pub(crate) fn read_config_value_from(path: &Path, name: &str) -> Option<String> {
    let contents = fs::read_to_string(path).ok()?;
    contents
        .lines()
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            (key.trim() == name).then(|| value.trim().to_owned())
        })
        .last()
}

pub(crate) fn read_config_value(name: &str) -> Option<String> {
    read_config_value_from(&config_path().ok()?, name)
}

pub(crate) fn configured_value(environment_name: &str, config_name: &str) -> Option<String> {
    env::var(environment_name)
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| read_config_value(config_name).filter(|value| !value.is_empty()))
}

pub(crate) fn is_configured() -> bool {
    configured_value("QN_TOKEN", "token").is_some()
        && configured_value("QN_ENDPOINT", "endpoint").is_some()
}

pub(crate) fn api_url_from_root(root: &str, path: &str) -> String {
    format!("{}/{path}", root.trim_end_matches('/'))
}

pub(crate) fn api_url(path: &str) -> Result<String, String> {
    let endpoint = configured_value("QN_ENDPOINT", "endpoint")
        .ok_or("未配置 QN_ENDPOINT，请运行 `qn init` 或设置环境变量".to_owned())?;
    Ok(api_url_from_root(&endpoint, path))
}

pub(crate) fn write_config_value(path: &Path, name: &str, value: &str) -> Result<(), String> {
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

pub(crate) fn macos_local_hostname() -> Option<String> {
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

pub(crate) fn select_default_device_name(
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

pub(crate) fn default_device_name() -> Result<String, String> {
    let hostname = hostname::get()
        .map_err(|error| format!("获取主机名失败: {error}"))?
        .to_string_lossy()
        .into_owned();
    select_default_device_name(hostname, macos_local_hostname())
}

pub(crate) fn device_name_from_config(path: &Path) -> Result<String, String> {
    if let Some(name) = read_config_value_from(path, "name").filter(|name| !name.is_empty()) {
        return Ok(name);
    }

    let name = default_device_name()?;
    write_config_value(path, "name", &name)?;
    Ok(name)
}

pub(crate) fn device_name() -> Result<String, String> {
    device_name_from_config(&config_path()?)
}

pub(crate) fn initialize_config_with_io(
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
pub(crate) fn initialize_config() -> Result<(), String> {
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
pub(crate) fn initialize_config() -> Result<(), String> {
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
