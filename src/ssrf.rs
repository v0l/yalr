use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum UrlRejected {
    #[error("base_url is not a valid URL")]
    Invalid,
    #[error("base_url must use http or https")]
    Scheme,
    #[error("base_url has no host")]
    NoHost,
    #[error("base_url host did not resolve")]
    Unresolved,
    #[error("base_url resolves to a non-public address ({0})")]
    NonPublic(IpAddr),
    #[error("base_url must not contain a user name or password")]
    Credentials,
    #[error("base_url must not contain a fragment")]
    Fragment,
    #[error("base_url port {0} is not allowed")]
    Port(u16),
}

const SERVICE_PORTS: &[u16] = &[
    2049, 2375, 2376, 2379, 2380, 3306, 4369, 5432, 5672, 5984, 6379, 6443, 8500, 9042, 9092, 9200, 9300, 10250,
    10255, 11211, 15672, 25672, 27017, 28017,
];

pub fn is_allowed_port(port: u16) -> bool {
    port == 80 || port == 443 || (port >= 1024 && !SERVICE_PORTS.contains(&port))
}

pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => is_public_v6(v6),
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_multicast()
        || a == 0
        || (a == 100 && (64..=127).contains(&b))
        || (a == 192 && b == 0 && ip.octets()[2] == 0)
        || (a == 198 && (b == 18 || b == 19))
        || a >= 240)
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return is_public_v4(v4);
    }
    let first = ip.segments()[0];
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_multicast()
        || (first & 0xfe00) == 0xfc00
        || (first & 0xffc0) == 0xfe80)
}

pub fn parse_public_url(raw: &str) -> Result<(url::Url, String, u16), UrlRejected> {
    let parsed = url::Url::parse(raw).map_err(|_| UrlRejected::Invalid)?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(UrlRejected::Scheme);
    }
    if parsed.fragment().is_some() || raw.contains('#') {
        return Err(UrlRejected::Fragment);
    }
    if !parsed.username().is_empty() || parsed.password().is_some() || raw.contains('@') {
        return Err(UrlRejected::Credentials);
    }
    let host = parsed.host_str().ok_or(UrlRejected::NoHost)?;
    let host = host.trim_start_matches('[').trim_end_matches(']').to_string();
    let port = parsed.port_or_known_default().ok_or(UrlRejected::Invalid)?;
    if !is_allowed_port(port) {
        return Err(UrlRejected::Port(port));
    }
    Ok((parsed, host, port))
}

pub fn checked_addrs(addrs: Vec<IpAddr>) -> Result<Vec<IpAddr>, UrlRejected> {
    if addrs.is_empty() {
        return Err(UrlRejected::Unresolved);
    }
    match addrs.iter().find(|ip| !is_public_ip(**ip)) {
        Some(ip) => Err(UrlRejected::NonPublic(*ip)),
        None => Ok(addrs),
    }
}

pub async fn system_lookup(host: &str, port: u16) -> Result<Vec<IpAddr>, UrlRejected> {
    match host.parse::<IpAddr>() {
        Ok(ip) => Ok(vec![ip]),
        Err(_) => Ok(tokio::net::lookup_host((host, port))
            .await
            .map_err(|_| UrlRejected::Unresolved)?
            .map(|a| a.ip())
            .collect()),
    }
}

pub async fn check_public_url(raw: &str) -> Result<(), UrlRejected> {
    let (_, host, port) = parse_public_url(raw)?;
    checked_addrs(system_lookup(&host, port).await?).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn rejected(url: &str) -> bool {
        matches!(check_public_url(url).await, Err(UrlRejected::NonPublic(_)))
    }

    #[tokio::test]
    async fn loopback_is_blocked() {
        assert!(rejected("http://127.0.0.1:8080/v1").await);
        assert!(rejected("http://127.5.5.5/v1").await);
        assert!(rejected("http://[::1]/v1").await);
        assert!(rejected("http://localhost/v1").await);
    }

    #[tokio::test]
    async fn rfc1918_is_blocked() {
        assert!(rejected("http://10.0.0.5/v1").await);
        assert!(rejected("http://172.16.0.1/v1").await);
        assert!(rejected("http://172.31.255.255/v1").await);
        assert!(rejected("http://192.168.1.1/v1").await);
    }

    #[tokio::test]
    async fn link_local_and_metadata_are_blocked() {
        assert!(rejected("http://169.254.169.254/latest/meta-data").await);
        assert!(rejected("http://169.254.1.1/").await);
        assert!(rejected("http://[fe80::1]/").await);
    }

    #[tokio::test]
    async fn cgnat_is_blocked() {
        assert!(rejected("http://100.64.0.1/").await);
        assert!(rejected("http://100.127.255.254/").await);
    }

    #[tokio::test]
    async fn unique_local_ipv6_is_blocked() {
        assert!(rejected("http://[fc00::1]/").await);
        assert!(rejected("http://[fd12:3456::1]/").await);
    }

    #[tokio::test]
    async fn mapped_ipv4_is_blocked() {
        assert!(rejected("http://[::ffff:127.0.0.1]/").await);
        assert!(rejected("http://[::ffff:10.0.0.1]/").await);
        assert!(rejected("http://[::ffff:169.254.169.254]/").await);
    }

    #[tokio::test]
    async fn unspecified_and_multicast_are_blocked() {
        assert!(rejected("http://0.0.0.0/").await);
        assert!(rejected("http://224.0.0.1/").await);
    }

    #[tokio::test]
    async fn public_addresses_pass() {
        assert!(check_public_url("https://8.8.8.8/v1").await.is_ok());
        assert!(check_public_url("http://[2606:4700:4700::1111]/v1").await.is_ok());
        assert!(check_public_url("http://100.63.255.255/").await.is_ok());
        assert!(check_public_url("http://172.32.0.1/").await.is_ok());
    }

    #[tokio::test]
    async fn parser_confusion_tricks_are_refused() {
        assert_eq!(check_public_url("https://example.com#@127.0.0.1/v1").await, Err(UrlRejected::Fragment));
        assert_eq!(check_public_url("https://8.8.8.8/v1#x").await, Err(UrlRejected::Fragment));
        for url in [
            "https://user@8.8.8.8/v1",
            "https://user:pw@8.8.8.8/v1",
            "http://example.com@127.0.0.1/",
            "http://8.8.8.8:80@10.0.0.1/",
            "http://127.0.0.1%2f@8.8.8.8/",
        ] {
            assert_eq!(check_public_url(url).await, Err(UrlRejected::Credentials), "{url}");
        }
    }

    #[tokio::test]
    async fn service_and_low_ports_are_refused() {
        for (url, port) in [
            ("https://example.com:22/v1", 22),
            ("http://8.8.8.8:6379/v1", 6379),
            ("http://8.8.8.8:5432/", 5432),
            ("http://8.8.8.8:3306/", 3306),
            ("http://8.8.8.8:11211/", 11211),
            ("http://8.8.8.8:9200/", 9200),
            ("http://8.8.8.8:2375/", 2375),
            ("http://8.8.8.8:10250/", 10250),
            ("http://8.8.8.8:25/", 25),
            ("http://0a2a0005.8ac540d4.rbndr.us:6379/v1", 6379),
        ] {
            assert_eq!(check_public_url(url).await, Err(UrlRejected::Port(port)), "{url}");
        }
        for port in [80, 443, 1234, 8000, 8080, 11434] {
            assert!(is_allowed_port(port), "{port}");
        }
        assert!(check_public_url("http://8.8.8.8:8000/v1").await.is_ok());
    }

    #[tokio::test]
    async fn numeric_host_spellings_of_loopback_are_blocked() {
        for url in ["http://2130706433/", "http://0x7f000001/", "http://0177.0.0.1/", "http://127.1/", "http://0/"] {
            assert!(rejected(url).await, "{url}");
        }
    }

    #[tokio::test]
    async fn other_schemes_are_refused() {
        assert_eq!(check_public_url("ftp://8.8.8.8/").await, Err(UrlRejected::Scheme));
        assert_eq!(check_public_url("file:///etc/passwd").await, Err(UrlRejected::Scheme));
        assert_eq!(check_public_url("not a url").await, Err(UrlRejected::Invalid));
    }
}
