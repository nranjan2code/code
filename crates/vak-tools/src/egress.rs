//! The loopback egress proxy a networked process reaches the network
//! through: an MCP server (docs/design/09-extensibility.md "Network") or a
//! skill's program (docs/design/93-self-evolving-capabilities.md §3). The
//! process runs inside the OS sandbox with this proxy's port as its one way
//! out; the proxy answers HTTP `CONNECT` for the hosts declared for it and
//! refuses everything else. It tunnels bytes and never reads them: what a
//! server sends inside TLS stays between it and the host it named.

use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

/// The longest request head the proxy reads before refusing it.
const MAX_HEAD: usize = 8 * 1024;

/// A running proxy, stopped when dropped.
pub struct Egress {
    port: u16,
    task: JoinHandle<()>,
}

impl Egress {
    /// Starts a proxy on a free loopback port for these declared hosts.
    pub async fn start(hosts: Vec<String>) -> std::io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let port = listener.local_addr()?.port();
        let hosts = Arc::new(hosts);
        let task = tokio::spawn(async move {
            while let Ok((client, _)) = listener.accept().await {
                let hosts = hosts.clone();
                tokio::spawn(async move {
                    let _ = serve(client, &hosts).await;
                });
            }
        });
        Ok(Self { port, task })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// The environment that points a server's HTTP clients at the proxy.
    /// Each runtime reads a different name, and Node's built-in `fetch`
    /// reads none unless told to.
    pub fn environment(&self) -> Vec<(String, String)> {
        let url = format!("http://127.0.0.1:{}", self.port);
        let mut env: Vec<(String, String)> = [
            "HTTPS_PROXY",
            "HTTP_PROXY",
            "ALL_PROXY",
            "https_proxy",
            "http_proxy",
            "all_proxy",
        ]
        .into_iter()
        .map(|name| (name.to_string(), url.clone()))
        .collect();
        env.push(("NO_PROXY".into(), String::new()));
        env.push(("no_proxy".into(), String::new()));
        env.push(("NODE_USE_ENV_PROXY".into(), "1".into()));
        env
    }
}

impl Drop for Egress {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Whether `host:port` is one of the declared hosts. A declared name allows
/// HTTPS and HTTP (443 and 80); a declared `name:port` allows that port.
/// Names compare exactly, ignoring case: there are no wildcards.
pub fn allowed(declared: &[String], host: &str, port: u16) -> bool {
    declared.iter().any(|entry| match entry.rsplit_once(':') {
        Some((name, entry_port)) => {
            name.eq_ignore_ascii_case(host) && entry_port.parse::<u16>().ok() == Some(port)
        }
        None => entry.eq_ignore_ascii_case(host) && matches!(port, 443 | 80),
    })
}

async fn serve(mut client: TcpStream, declared: &[String]) -> std::io::Result<()> {
    let mut head = Vec::new();
    let mut buf = [0u8; 1024];
    let end = loop {
        let n = client.read(&mut buf).await?;
        if n == 0 {
            return Ok(());
        }
        head.extend_from_slice(&buf[..n]);
        if let Some(at) = head.windows(4).position(|window| window == b"\r\n\r\n") {
            break at + 4;
        }
        if head.len() > MAX_HEAD {
            return refuse(&mut client, "431 Request Header Fields Too Large").await;
        }
    };
    let line = String::from_utf8_lossy(&head[..end]);
    let target = line.lines().next().and_then(|first| {
        let mut parts = first.split_whitespace();
        match (parts.next(), parts.next()) {
            (Some("CONNECT"), Some(target)) => Some(target.to_string()),
            _ => None,
        }
    });
    let Some(target) = target else {
        tracing::info!(
            kind = "mcp_egress",
            outcome = "refused",
            "only CONNECT is proxied"
        );
        return refuse(&mut client, "405 Method Not Allowed").await;
    };
    let Some((host, port)) = target
        .rsplit_once(':')
        .and_then(|(host, port)| Some((host.trim_matches(['[', ']']), port.parse::<u16>().ok()?)))
    else {
        return refuse(&mut client, "400 Bad Request").await;
    };
    if !allowed(declared, host, port) {
        tracing::info!(
            kind = "mcp_egress",
            outcome = "refused",
            "a host not declared for the server"
        );
        return refuse(&mut client, "403 Forbidden").await;
    }
    let mut upstream = match TcpStream::connect((host, port)).await {
        Ok(stream) => stream,
        Err(_) => return refuse(&mut client, "502 Bad Gateway").await,
    };
    client
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .await?;
    // Bytes a client sent after the head without waiting for the reply.
    if head.len() > end {
        upstream.write_all(&head[end..]).await?;
    }
    tokio::io::copy_bidirectional(&mut client, &mut upstream).await?;
    Ok(())
}

async fn refuse(client: &mut TcpStream, status: &str) -> std::io::Result<()> {
    client
        .write_all(
            format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .await
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn a_declared_name_allows_https_and_http_only() {
        let declared = vec![
            "api.example.com".to_string(),
            "Registry.Example.org:8443".to_string(),
        ];
        assert!(allowed(&declared, "api.example.com", 443));
        assert!(allowed(&declared, "API.EXAMPLE.COM", 80));
        assert!(!allowed(&declared, "api.example.com", 22));
        assert!(!allowed(&declared, "evil.example.com", 443));
        assert!(!allowed(&declared, "sub.api.example.com", 443));
        assert!(allowed(&declared, "registry.example.org", 8443));
        assert!(!allowed(&declared, "registry.example.org", 443));
    }

    async fn ask(port: u16, request: &str) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("connect");
        stream.write_all(request.as_bytes()).await.expect("write");
        let mut reply = vec![0u8; 256];
        let n = stream.read(&mut reply).await.expect("read");
        String::from_utf8_lossy(&reply[..n]).to_string()
    }

    #[tokio::test]
    async fn a_host_not_declared_is_refused() {
        let egress = Egress::start(vec!["api.example.com".into()])
            .await
            .expect("start");
        let reply = ask(
            egress.port(),
            "CONNECT evil.example.com:443 HTTP/1.1\r\n\r\n",
        )
        .await;
        assert!(reply.starts_with("HTTP/1.1 403"), "{reply}");
    }

    #[tokio::test]
    async fn a_plain_request_is_refused() {
        let egress = Egress::start(vec!["api.example.com".into()])
            .await
            .expect("start");
        let reply = ask(
            egress.port(),
            "GET http://api.example.com/ HTTP/1.1\r\nHost: api.example.com\r\n\r\n",
        )
        .await;
        assert!(reply.starts_with("HTTP/1.1 405"), "{reply}");
    }

    #[tokio::test]
    async fn a_declared_host_is_tunnelled() {
        // A local listener stands in for the remote host.
        let upstream = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
        let upstream_port = upstream.local_addr().expect("addr").port();
        tokio::spawn(async move {
            if let Ok((mut socket, _)) = upstream.accept().await {
                let mut buf = [0u8; 4];
                if socket.read_exact(&mut buf).await.is_ok() {
                    let _ = socket.write_all(b"pong").await;
                }
            }
        });
        let egress = Egress::start(vec![format!("127.0.0.1:{upstream_port}")])
            .await
            .expect("start");
        let mut stream = TcpStream::connect(("127.0.0.1", egress.port()))
            .await
            .expect("connect");
        stream
            .write_all(format!("CONNECT 127.0.0.1:{upstream_port} HTTP/1.1\r\n\r\n").as_bytes())
            .await
            .expect("write");
        let mut reply = vec![0u8; 39];
        stream.read_exact(&mut reply).await.expect("reply");
        assert!(String::from_utf8_lossy(&reply).starts_with("HTTP/1.1 200"));
        stream.write_all(b"ping").await.expect("ping");
        let mut pong = [0u8; 4];
        stream.read_exact(&mut pong).await.expect("pong");
        assert_eq!(&pong, b"pong");
    }
}
