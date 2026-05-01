use axum::{
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use governor::{
    clock::DefaultClock,
    state::{InMemoryState, NotKeyed},
    Quota, RateLimiter,
};
use moka::sync::Cache;
use std::{
    net::{IpAddr, SocketAddr},
    num::NonZeroU32,
    sync::Arc,
};

const RATE_LIMIT_REQUESTS_PER_SECOND: u32 = 100;
const X_FORWARDED_FOR_HEADER: &str = "x-forwarded-for";

// Hard upper bound on the number of distinct IPs the limiter will track at any time.
// Memory footprint is O(1) regardless of incoming IP diversity, eliminating the
// IP-spoofing OOM vector that an unbounded `DefaultKeyedRateLimiter` would expose.
const MAX_TRACKED_IPS: u64 = 100_000;

type IpRateLimiter = RateLimiter<NotKeyed, InMemoryState, DefaultClock>;

pub(super) struct BoundedRateLimiter {
    cache: Cache<IpAddr, Arc<IpRateLimiter>>,
    quota: Quota,
}

impl BoundedRateLimiter {
    fn new(quota: Quota, max_entries: u64) -> Self {
        Self {
            cache: Cache::builder().max_capacity(max_entries).build(),
            quota,
        }
    }

    fn check(&self, ip: IpAddr) -> bool {
        let limiter = self
            .cache
            .get_with(ip, || Arc::new(RateLimiter::direct(self.quota)));
        limiter.check().is_ok()
    }

    #[cfg(test)]
    fn entry_count(&self) -> u64 {
        // moka's run_pending_tasks promotes the in-memory size after recent inserts.
        self.cache.run_pending_tasks();
        self.cache.entry_count()
    }
}

pub(super) type SharedRateLimiter = Arc<BoundedRateLimiter>;

pub(super) fn new_rate_limiter() -> SharedRateLimiter {
    let quota = Quota::per_second(
        NonZeroU32::new(RATE_LIMIT_REQUESTS_PER_SECOND)
            .expect("rate limit quota should be non-zero"),
    );
    Arc::new(BoundedRateLimiter::new(quota, MAX_TRACKED_IPS))
}

pub(super) async fn rate_limit_middleware(
    State(limiter): State<SharedRateLimiter>,
    req: Request,
    next: Next,
) -> Response {
    if let Some(client_ip) = resolve_client_ip(
        req.headers(),
        req.extensions().get::<ConnectInfo<SocketAddr>>(),
    ) {
        if !limiter.check(client_ip) {
            return StatusCode::TOO_MANY_REQUESTS.into_response();
        }
    }

    next.run(req).await
}

fn resolve_client_ip(
    headers: &HeaderMap,
    connect_info: Option<&ConnectInfo<SocketAddr>>,
) -> Option<IpAddr> {
    forwarded_for_ip(headers).or_else(|| connect_info.map(|connect_info| connect_info.0.ip()))
}

fn forwarded_for_ip(headers: &HeaderMap) -> Option<IpAddr> {
    headers
        .get(X_FORWARDED_FOR_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .and_then(parse_ip_candidate)
}

fn parse_ip_candidate(candidate: &str) -> Option<IpAddr> {
    candidate
        .parse::<IpAddr>()
        .ok()
        .or_else(|| candidate.parse::<SocketAddr>().ok().map(|addr| addr.ip()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request, middleware::from_fn_with_state, routing::get, Router};
    use std::net::Ipv4Addr;
    use tower::util::ServiceExt;

    #[test]
    fn resolve_client_ip_prefers_forwarded_for_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            X_FORWARDED_FOR_HEADER,
            "203.0.113.10, 198.51.100.20"
                .parse()
                .expect("header should be valid"),
        );
        let connect_info = ConnectInfo(SocketAddr::from(([10, 0, 0, 5], 8080)));

        assert_eq!(
            resolve_client_ip(&headers, Some(&connect_info)),
            Some(IpAddr::from([203, 0, 113, 10]))
        );
    }

    #[tokio::test]
    async fn middleware_returns_too_many_requests_after_quota_is_exhausted() {
        let quota = Quota::per_minute(
            NonZeroU32::new(2).expect("test rate limit quota should be non-zero"),
        );
        let app = Router::new()
            .route("/", get(|| async { StatusCode::OK }))
            .layer(from_fn_with_state(
                Arc::new(BoundedRateLimiter::new(quota, MAX_TRACKED_IPS)),
                rate_limit_middleware,
            ));

        for _ in 0..2 {
            let response = app
                .clone()
                .oneshot(
                    Request::get("/")
                        .header(X_FORWARDED_FOR_HEADER, "203.0.113.10")
                        .body(Body::empty())
                        .expect("request should build"),
                )
                .await
                .expect("request should complete");

            assert_eq!(response.status(), StatusCode::OK);
        }

        let response = app
            .oneshot(
                Request::get("/")
                    .header(X_FORWARDED_FOR_HEADER, "203.0.113.10")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[test]
    fn limiter_caps_tracked_ip_set_under_spoofing_pressure() {
        // Build a limiter with a tiny cap so we can prove the bound is enforced
        // without spending the time/memory of inserting 100k entries.
        let small_cap: u64 = 1024;
        let quota = Quota::per_second(
            NonZeroU32::new(RATE_LIMIT_REQUESTS_PER_SECOND)
                .expect("rate limit requests per second must be non-zero"),
        );
        let limiter = BoundedRateLimiter::new(quota, small_cap);

        // Hammer with 10x the cap of distinct synthetic IPs.
        let total: u32 = (small_cap * 10) as u32;
        for n in 0..total {
            let ip = IpAddr::V4(Ipv4Addr::from(n));
            assert!(limiter.check(ip));
        }

        let count = limiter.entry_count();
        assert!(
            count <= small_cap,
            "tracked IP set ({count}) must not exceed cap ({small_cap})"
        );
    }
}
