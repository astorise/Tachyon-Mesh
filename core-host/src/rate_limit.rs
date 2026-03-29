use axum::{
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use governor::{DefaultKeyedRateLimiter, Quota, RateLimiter};
use std::{
    net::{IpAddr, SocketAddr},
    num::NonZeroU32,
    sync::Arc,
};

const RATE_LIMIT_REQUESTS_PER_SECOND: u32 = 100;
const X_FORWARDED_FOR_HEADER: &str = "x-forwarded-for";

pub(super) type SharedRateLimiter = Arc<DefaultKeyedRateLimiter<IpAddr>>;

pub(super) fn new_rate_limiter() -> SharedRateLimiter {
    Arc::new(RateLimiter::keyed(Quota::per_second(
        NonZeroU32::new(RATE_LIMIT_REQUESTS_PER_SECOND)
            .expect("rate limit quota should be non-zero"),
    )))
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
        if limiter.check_key(&client_ip).is_err() {
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
        let app = Router::new()
            .route("/", get(|| async { StatusCode::OK }))
            .layer(from_fn_with_state(
                new_rate_limiter(),
                rate_limit_middleware,
            ));

        for _ in 0..RATE_LIMIT_REQUESTS_PER_SECOND {
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
}
