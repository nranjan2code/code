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

fn local_admin_url() -> Result<String, String> {
    let connection = vak_client::GatewayConnection::discover_local()
        .map_err(|error| format!("no usable local gateway: {error}"))?;
    Ok(format!(
        "{}/admin#token={}",
        connection.base_url(),
        encode_fragment(connection.token())
    ))
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
    let url = match local_admin_url() {
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
