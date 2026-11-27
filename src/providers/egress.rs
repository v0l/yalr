use std::future::Future;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use crate::ssrf::{is_public_ip, UrlRejected};

type BoxError = Box<dyn std::error::Error + Send + Sync>;
type Addrs = Box<dyn Iterator<Item = SocketAddr> + Send>;
type Resolving = Pin<Box<dyn Future<Output = Result<Addrs, BoxError>> + Send>>;

#[async_trait::async_trait]
pub trait Lookup: Send + Sync {
    async fn lookup(&self, host: &str) -> std::io::Result<Vec<IpAddr>>;
}

pub struct SystemLookup;

#[async_trait::async_trait]
impl Lookup for SystemLookup {
    async fn lookup(&self, host: &str) -> std::io::Result<Vec<IpAddr>> {
        Ok(tokio::net::lookup_host((host, 0)).await?.map(|a| a.ip()).collect())
    }
}

#[derive(Clone)]
pub struct Pinned {
    lookup: Arc<dyn Lookup>,
    allow: fn(IpAddr) -> bool,
}

impl Pinned {
    pub async fn checked(&self, host: &str) -> Result<Vec<IpAddr>, UrlRejected> {
        let host = host.trim_start_matches('[').trim_end_matches(']');
        let found = match host.parse::<IpAddr>() {
            Ok(ip) => vec![ip],
            Err(_) => self.lookup.lookup(host).await.map_err(|_| UrlRejected::Unresolved)?,
        };
        if found.is_empty() {
            return Err(UrlRejected::Unresolved);
        }
        match found.iter().find(|ip| !(self.allow)(**ip)) {
            Some(ip) => Err(UrlRejected::NonPublic(*ip)),
            None => Ok(found),
        }
    }

    fn resolving(&self, host: String) -> Resolving {
        let pinned = self.clone();
        Box::pin(async move {
            let found = pinned.checked(&host).await.map_err(|e| Box::new(e) as BoxError)?;
            Ok(Box::new(found.into_iter().map(|ip| SocketAddr::new(ip, 0))) as Addrs)
        })
    }
}

impl reqwest::dns::Resolve for Pinned {
    fn resolve(&self, name: hyper::client::connect::dns::Name) -> reqwest::dns::Resolving {
        self.resolving(name.as_str().to_string())
    }
}

impl reqwest_openai::dns::Resolve for Pinned {
    fn resolve(&self, name: reqwest_openai::dns::Name) -> reqwest_openai::dns::Resolving {
        self.resolving(name.as_str().to_string())
    }
}

#[derive(Clone)]
pub struct Egress {
    pinned: Pinned,
    connect_timeout: Duration,
}

#[derive(Clone)]
pub struct Clients {
    pub http: reqwest::Client,
    pub api: reqwest_openai::Client,
}

impl Default for Egress {
    fn default() -> Self {
        Self::with_lookup(Arc::new(SystemLookup), is_public_ip)
    }
}

impl Egress {
    pub fn with_lookup(lookup: Arc<dyn Lookup>, allow: fn(IpAddr) -> bool) -> Self {
        Self { pinned: Pinned { lookup, allow }, connect_timeout: Duration::from_secs(8) }
    }

    pub fn pinned(&self) -> &Pinned {
        &self.pinned
    }

    pub fn http(&self) -> reqwest::Client {
        reqwest::Client::builder()
            .dns_resolver(Arc::new(self.pinned.clone()))
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .connect_timeout(self.connect_timeout)
            .pool_idle_timeout(Duration::from_secs(30))
            .build()
            .expect("pinned http client")
    }

    pub fn api(&self) -> reqwest_openai::Client {
        reqwest_openai::Client::builder()
            .dns_resolver(Arc::new(self.pinned.clone()))
            .redirect(reqwest_openai::redirect::Policy::none())
            .no_proxy()
            .connect_timeout(self.connect_timeout)
            .pool_idle_timeout(Duration::from_secs(30))
            .build()
            .expect("pinned api client")
    }

    pub fn clients(&self) -> Clients {
        Clients { http: self.http(), api: self.api() }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Caller {
    pub user_id: i64,
    pub is_admin: bool,
    pub verify: bool,
}

impl Caller {
    pub fn user(user_id: i64, is_admin: bool) -> Self {
        Self { user_id, is_admin, verify: false }
    }

    pub fn verification(owner: i64) -> Self {
        Self { user_id: owner, is_admin: false, verify: true }
    }
}

tokio::task_local! {
    static CALLER: Caller;
}

pub async fn as_caller<F: Future>(caller: Caller, work: F) -> F::Output {
    CALLER.scope(caller, work).await
}

pub fn caller() -> Option<Caller> {
    CALLER.try_with(|c| *c).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    pub(crate) struct Flipping(pub(crate) Mutex<Vec<Vec<IpAddr>>>, pub(crate) Mutex<Vec<String>>);

    impl Flipping {
        pub(crate) fn new(answers: &[&[&str]]) -> Arc<Self> {
            let answers = answers.iter().map(|a| a.iter().map(|ip| ip.parse().unwrap()).collect()).collect();
            Arc::new(Self(Mutex::new(answers), Mutex::new(vec![])))
        }
    }

    #[async_trait::async_trait]
    impl Lookup for Flipping {
        async fn lookup(&self, host: &str) -> std::io::Result<Vec<IpAddr>> {
            self.1.lock().unwrap().push(host.to_string());
            let mut answers = self.0.lock().unwrap();
            Ok(if answers.len() > 1 { answers.remove(0) } else { answers[0].clone() })
        }
    }

    pub(crate) fn loopback_is_public(ip: IpAddr) -> bool {
        ip == IpAddr::from([127, 0, 0, 1])
    }

    fn port(server: &MockServer) -> u16 {
        server.address().port()
    }

    #[tokio::test]
    async fn the_client_connects_to_the_checked_answer_not_the_system_resolver() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(200)).mount(&server).await;
        let lookup = Flipping::new(&[&["127.0.0.1"]]);
        let egress = Egress::with_lookup(lookup.clone(), loopback_is_public);
        let url = format!("http://upstream.invalid:{}/v1/models", port(&server));
        assert_eq!(egress.http().get(&url).send().await.unwrap().status(), 200);
        assert_eq!(egress.api().get(&url).send().await.unwrap().status(), 200);
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
        assert!(lookup.1.lock().unwrap().iter().all(|h| h == "upstream.invalid"));
    }

    #[tokio::test]
    async fn a_name_that_rebinds_to_a_private_address_is_never_connected_to() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(200)).mount(&server).await;
        let lookup = Flipping::new(&[&["127.0.0.1"], &["10.42.0.5"]]);
        let egress = Egress::with_lookup(lookup.clone(), loopback_is_public);
        assert!(egress.pinned().checked("rebind.invalid").await.is_ok());
        let url = format!("http://rebind.invalid:{}/v1/models", port(&server));
        let err = egress.http().get(&url).send().await.unwrap_err();
        assert!(format!("{err:?}").contains("NonPublic(10.42.0.5)"), "{err:?}");
        let err = egress.api().get(&url).send().await.unwrap_err();
        assert!(format!("{err:?}").contains("NonPublic(10.42.0.5)"), "{err:?}");
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_mixed_answer_is_refused_whole() {
        let egress = Egress::with_lookup(Flipping::new(&[&["127.0.0.1", "10.0.0.1"]]), loopback_is_public);
        assert_eq!(egress.pinned().checked("mixed.invalid").await, Err(UrlRejected::NonPublic("10.0.0.1".parse().unwrap())));
    }

    #[tokio::test]
    async fn a_redirect_to_another_address_is_not_followed() {
        let target = MockServer::start().await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(200)).mount(&target).await;
        let source = MockServer::start().await;
        let location = format!("http://127.0.0.1:{}/internal", port(&target));
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(302).insert_header("location", location.as_str()))
            .mount(&source)
            .await;
        let egress = Egress::with_lookup(Flipping::new(&[&["127.0.0.1"]]), loopback_is_public);
        let url = format!("http://source.invalid:{}/v1/models", port(&source));
        assert_eq!(egress.http().get(&url).send().await.unwrap().status(), 302);
        assert_eq!(egress.api().get(&url).send().await.unwrap().status(), 302);
        assert!(target.received_requests().await.unwrap().is_empty());
    }
}
