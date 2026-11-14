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
    let host = parsed.host_str().ok_or(UrlRejected::NoHost)?.to_string();
    let port = parsed.port_or_known_default().ok_or(UrlRejected::Invalid)?;
    Ok((parsed, host, port))
}

pub async fn check_public_url(raw: &str) -> Result<(), UrlRejected> {
    let (_, host, port) = parse_public_url(raw)?;
    let host = host.trim_start_matches('[').trim_end_matches(']').to_string();
    let addrs: Vec<IpAddr> = match host.parse::<IpAddr>() {
        Ok(ip) => vec![ip],
        Err(_) => tokio::net::lookup_host((host.as_str(), port))
            .await
            .map_err(|_| UrlRejected::Unresolved)?
            .map(|a| a.ip())
            .collect(),
    };
    if addrs.is_empty() {
        return Err(UrlRejected::Unresolved);
    }
    match addrs.into_iter().find(|ip| !is_public_ip(*ip)) {
        Some(ip) => Err(UrlRejected::NonPublic(ip)),
        None => Ok(()),
    }
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
    async fn other_schemes_are_refused() {
        assert_eq!(check_public_url("ftp://8.8.8.8/").await, Err(UrlRejected::Scheme));
        assert_eq!(check_public_url("file:///etc/passwd").await, Err(UrlRejected::Scheme));
        assert_eq!(check_public_url("not a url").await, Err(UrlRejected::Invalid));
    }
}
