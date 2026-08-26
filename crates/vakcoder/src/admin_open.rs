use std::path::Path;

fn pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }
    #[cfg(windows)]
    {
        std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .is_ok_and(|output| {
                output.status.success()
                    && String::from_utf8_lossy(&output.stdout).contains(&pid.to_string())
            })
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        false
    }
}

fn encode_fragment(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

fn url_from_runtime(path: &Path) -> Result<String, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|_| "no local gateway runtime found; start the gateway first".to_string())?;
    let value: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| format!("gateway runtime is invalid: {e}"))?;
    let pid = value
        .get("pid")
        .and_then(serde_json::Value::as_u64)
        .and_then(|pid| u32::try_from(pid).ok())
        .ok_or_else(|| "gateway runtime has no valid pid".to_string())?;
    if !pid_alive(pid) {
        return Err("gateway runtime is stale; start the gateway first".to_string());
    }
    let addr = value
        .get("addr")
        .and_then(serde_json::Value::as_str)
        .filter(|addr| !addr.is_empty())
        .ok_or_else(|| "gateway runtime has no address".to_string())?;
    let token = value
        .get("token")
        .and_then(serde_json::Value::as_str)
        .filter(|token| !token.is_empty())
        .ok_or_else(|| "gateway runtime has no token".to_string())?;
    let base = if addr.starts_with("http://") || addr.starts_with("https://") {
        addr.to_string()
    } else {
        format!("http://{addr}")
    };
    Ok(format!("{base}/admin#token={}", encode_fragment(token)))
}

fn open_browser(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(target_os = "linux")]
    let mut command = std::process::Command::new("xdg-open");
    #[cfg(windows)]
    let mut command = std::process::Command::new("explorer.exe");
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    return Err(format!("browser opening is unsupported; open {url}"));

    command
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("could not open browser: {e}; open {url}"))
}

pub(crate) fn run(print: bool) -> i32 {
    let path = vak_config::paths::data_home()
        .join("runtime")
        .join("gateway.json");
    let url = match url_from_runtime(&path) {
        Ok(url) => url,
        Err(error) => {
            eprintln!("error: {error}");
            return 1;
        }
    };
    if print {
        println!("{url}");
        return 0;
    }
    match open_browser(&url) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("error: {error}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fragment_encoding_keeps_tokens_out_of_request_paths() {
        assert_eq!(encode_fragment("vk_a/b+c"), "vk_a%2Fb%2Bc");
    }
}
