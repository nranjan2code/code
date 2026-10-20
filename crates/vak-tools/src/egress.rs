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
            let mut connections = tokio::task::JoinSet::new();
            while let Ok((client, _)) = listener.accept().await {
                let hosts = hosts.clone();
                connections.spawn(async move {
                    let _ = serve(client, &hosts).await;
                });
                while connections.try_join_next().is_some() {}
            }
        });
        Ok(Self { port, task })
    }

    /// A browser's sole network path: SOCKS5 with address screening and
    /// pinned connections for every navigation, redirect and subresource.
    pub async fn screened_browser() -> std::io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let port = listener.local_addr()?.port();
        let task = tokio::spawn(async move {
            let mut connections = tokio::task::JoinSet::new();
            while let Ok((client, _)) = listener.accept().await {
                connections.spawn(async move {
                    let _ = serve_socks(client).await;
                });
                while connections.try_join_next().is_some() {}
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

async fn serve_socks(mut client: TcpStream) -> std::io::Result<()> {
    let mut greeting = [0u8; 2];
    client.read_exact(&mut greeting).await?;
    if greeting[0] != 5 {
        return Ok(());
    }
    let mut methods = vec![0; greeting[1] as usize];
    client.read_exact(&mut methods).await?;
    if !methods.contains(&0) {
        client.write_all(&[5, 255]).await?;
        return Ok(());
    }
    client.write_all(&[5, 0]).await?;
    let mut request = [0u8; 4];
    client.read_exact(&mut request).await?;
    if request[..3] != [5, 1, 0] {
        return Ok(());
    }
    let host = match request[3] {
        1 => {
            let mut address = [0; 4];
            client.read_exact(&mut address).await?;
            std::net::Ipv4Addr::from(address).to_string()
        }
        4 => {
            let mut address = [0; 16];
            client.read_exact(&mut address).await?;
            std::net::Ipv6Addr::from(address).to_string()
        }
        3 => {
            let size = client.read_u8().await? as usize;
            let mut address = vec![0; size];
            client.read_exact(&mut address).await?;
            String::from_utf8(address).map_err(|_| std::io::Error::other("invalid SOCKS host"))?
        }
        _ => return Ok(()),
    };
    let port = client.read_u16().await?;
    let addresses = match crate::webfetch::screened_addresses(&host, port).await {
        Ok(addresses) => addresses,
        Err(_) => {
            client.write_all(&[5, 2, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
            return Ok(());
        }
    };
    // Connecting to SocketAddr never resolves the hostname a second time.
    let mut upstream = TcpStream::connect(addresses.as_slice()).await?;
    client.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
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

#[cfg(test)]
mod screened_browser_tests {
    use super::*;

    #[tokio::test]
    async fn every_socks_connection_refuses_private_literal_and_dns_destinations() {
        let proxy = Egress::screened_browser().await.unwrap();
        for host in [
            "127.0.0.1",
            "localhost",
            "169.254.169.254",
            "10.0.0.1",
            "::1",
            "::ffff:127.0.0.1",
        ] {
            let mut stream = TcpStream::connect(("127.0.0.1", proxy.port()))
                .await
                .unwrap();
            stream.write_all(&[5, 1, 0]).await.unwrap();
            let mut method = [0; 2];
            stream.read_exact(&mut method).await.unwrap();
            assert_eq!(method, [5, 0]);
            let mut request = vec![5, 1, 0, 3, host.len() as u8];
            request.extend(host.as_bytes());
            request.extend(80u16.to_be_bytes());
            stream.write_all(&request).await.unwrap();
            let mut reply = [0; 10];
            stream.read_exact(&mut reply).await.unwrap();
            assert_eq!(reply[1], 2, "{host}");
        }
    }
}
