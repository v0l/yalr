use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::http::StatusCode;

use crate::api::tenant::TenantError;

pub const CREATES_PER_DAY: usize = 5;
pub const CREATES_PER_IP_PER_DAY: usize = 10;
pub const PENDING_PER_USER: i64 = 3;
pub const OWNER_CONTACTS_PER_HOUR: usize = 5;

const DAY: Duration = Duration::from_secs(24 * 60 * 60);
const HOUR: Duration = Duration::from_secs(60 * 60);

#[derive(Default)]
pub struct Window {
    hits: Mutex<HashMap<String, Vec<Instant>>>,
}

impl Window {
    pub fn allow(&self, key: &str, limit: usize, span: Duration) -> bool {
        let mut hits = self.hits.lock().unwrap();
        hits.retain(|_, times| times.iter().any(|t| t.elapsed() < span));
        let times = hits.entry(key.to_string()).or_default();
        times.retain(|t| t.elapsed() < span);
        if times.len() >= limit {
            return false;
        }
        times.push(Instant::now());
        true
    }
}

#[derive(Default)]
pub struct ProviderLimits {
    creates_by_user: Window,
    creates_by_ip: Window,
    contacts_by_user: Window,
}

fn too_many(message: &str) -> TenantError {
    (StatusCode::TOO_MANY_REQUESTS, message.to_string())
}

impl ProviderLimits {
    pub fn create(&self, user_id: i64, ip: &str) -> Result<(), TenantError> {
        if !self.creates_by_ip.allow(ip, CREATES_PER_IP_PER_DAY, DAY) {
            return Err(too_many("Too many providers added from this address today, try again tomorrow"));
        }
        if !self.creates_by_user.allow(&user_id.to_string(), CREATES_PER_DAY, DAY) {
            return Err(too_many("You can add 5 providers a day, try again tomorrow"));
        }
        Ok(())
    }

    pub fn contact(&self, user_id: i64) -> Result<(), TenantError> {
        if !self.contacts_by_user.allow(&user_id.to_string(), OWNER_CONTACTS_PER_HOUR, HOUR) {
            return Err(too_many("You can check or verify unapproved providers 5 times an hour, try again later"));
        }
        Ok(())
    }
}

pub fn client_ip(headers: &axum::http::HeaderMap, peer: Option<std::net::SocketAddr>) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .or_else(|| peer.map(|p| p.ip().to_string()))
        .unwrap_or_else(|| "unknown".to_string())
}

pub async fn owner_contact<F: std::future::Future>(
    state: &crate::state::AppState,
    scope: &crate::api::tenant::Scope,
    provider: &crate::db::Provider,
    work: F,
) -> Result<F::Output, TenantError> {
    if provider.owner_user_id.is_some() && provider.status != crate::db::APPROVED && !scope.is_admin() {
        state.config.router.abuse_limits.contact(scope.user.id)?;
    }
    let caller = crate::providers::egress::Caller::user(scope.user.id, scope.is_admin());
    Ok(crate::providers::egress::as_caller(caller, work).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_admits_up_to_its_limit_per_key() {
        let w = Window::default();
        for _ in 0..3 {
            assert!(w.allow("a", 3, HOUR));
        }
        assert!(!w.allow("a", 3, HOUR));
        assert!(w.allow("b", 3, HOUR));
        assert!(w.allow("a", 3, Duration::ZERO));
    }

    #[test]
    fn creates_are_capped_per_user_and_per_address() {
        let limits = ProviderLimits::default();
        for i in 0..CREATES_PER_DAY {
            limits.create(1, &format!("1.1.1.{i}")).unwrap();
        }
        assert_eq!(limits.create(1, "2.2.2.2").unwrap_err().0, StatusCode::TOO_MANY_REQUESTS);
        for user in 0..CREATES_PER_IP_PER_DAY as i64 {
            limits.create(100 + user, "3.3.3.3").unwrap();
        }
        assert_eq!(limits.create(9, "3.3.3.3").unwrap_err().0, StatusCode::TOO_MANY_REQUESTS);
        limits.create(9, "4.4.4.4").unwrap();
    }
}
