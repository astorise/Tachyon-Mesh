#![cfg(feature = "resiliency")]

use super::{
    execute_route_with_middleware_inner, ResiliencyConfig, RouteExecutionResult, RouteInvocation,
    RouteServiceError,
};
use axum::http::StatusCode;
use std::{collections::BTreeSet, future, time::Duration};
use tower::util::ServiceExt;
use tower::{
    retry::{Policy, RetryLayer},
    service_fn,
    timeout::{error::Elapsed, TimeoutLayer},
    BoxError, Layer,
};

#[derive(Clone, Debug)]
struct StatusRetryPolicy {
    remaining_retries: u32,
    retry_on: BTreeSet<u16>,
}

impl StatusRetryPolicy {
    fn new(config: &super::RetryPolicy) -> Self {
        Self {
            remaining_retries: config.max_retries,
            retry_on: config.retry_on.iter().copied().collect(),
        }
    }

    fn should_retry_status(&self, status: StatusCode) -> bool {
        self.retry_on.contains(&status.as_u16())
    }
}

impl<Req, E> Policy<Req, RouteExecutionResult, E> for StatusRetryPolicy
where
    Req: Clone,
{
    type Future = future::Ready<()>;

    fn retry(
        &mut self,
        _request: &mut Req,
        result: &mut Result<RouteExecutionResult, E>,
    ) -> Option<Self::Future> {
        match result {
            Ok(response)
                if self.remaining_retries > 0
                    && self.should_retry_status(response.response.status) =>
            {
                self.remaining_retries -= 1;
                Some(future::ready(()))
            }
            _ => None,
        }
    }

    fn clone_request(&mut self, request: &Req) -> Option<Req> {
        Some(request.clone())
    }
}

pub(crate) async fn execute_route_with_resiliency(
    invocation: RouteInvocation,
) -> std::result::Result<RouteExecutionResult, (StatusCode, String)> {
    let route_path = invocation.route.path.clone();
    let config = invocation.route.resiliency.clone();
    let service = service_fn(|request: RouteInvocation| async move {
        execute_route_with_middleware_inner(&request)
            .await
            .map_err(RouteServiceError::from)
    });

    call_with_resiliency(service, invocation, &route_path, config.as_ref()).await
}

pub(crate) async fn call_with_resiliency<S, Request>(
    service: S,
    request: Request,
    route_path: &str,
    config: Option<&ResiliencyConfig>,
) -> std::result::Result<RouteExecutionResult, (StatusCode, String)>
where
    Request: Clone + Send + 'static,
    S: tower::Service<Request, Response = RouteExecutionResult, Error = RouteServiceError>
        + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
{
    let Some(config) = config else {
        return service.oneshot(request).await.map_err(|error| error.into());
    };

    let timeout = config.timeout_ms.map(Duration::from_millis);
    let retry_policy = config.retry_policy.as_ref().and_then(|policy| {
        (policy.max_retries > 0 && !policy.retry_on.is_empty())
            .then(|| StatusRetryPolicy::new(policy))
    });

    match (timeout, retry_policy) {
        (Some(timeout), Some(policy)) => RetryLayer::new(policy)
            .layer(TimeoutLayer::new(timeout).layer(service))
            .oneshot(request)
            .await
            .map_err(|error| map_box_error(error, route_path, timeout)),
        (Some(timeout), None) => TimeoutLayer::new(timeout)
            .layer(service)
            .oneshot(request)
            .await
            .map_err(|error| map_box_error(error, route_path, timeout)),
        (None, Some(policy)) => RetryLayer::new(policy)
            .layer(service)
            .oneshot(request)
            .await
            .map_err(|error| error.into()),
        (None, None) => service.oneshot(request).await.map_err(|error| error.into()),
    }
}

fn map_box_error(error: BoxError, route_path: &str, timeout: Duration) -> (StatusCode, String) {
    if error.downcast_ref::<Elapsed>().is_some() {
        return (
            StatusCode::GATEWAY_TIMEOUT,
            format!(
                "route `{route_path}` timed out after {}ms",
                timeout.as_millis()
            ),
        );
    }

    if let Some(route_error) = error.downcast_ref::<RouteServiceError>() {
        return route_error.clone().into();
    }

    (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("route `{route_path}` resiliency middleware failed: {error}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    fn response(status: StatusCode, body: &str) -> RouteExecutionResult {
        RouteExecutionResult {
            response: super::super::GuestHttpResponse {
                status,
                body: Bytes::from(body.to_owned()),
            },
            fuel_consumed: None,
        }
    }

    #[tokio::test]
    async fn retries_configured_statuses_until_success() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let service = service_fn({
            let attempts = Arc::clone(&attempts);
            move |_: ()| {
                let attempts = Arc::clone(&attempts);
                async move {
                    let attempt = attempts.fetch_add(1, Ordering::SeqCst);
                    if attempt < 2 {
                        Ok(response(StatusCode::SERVICE_UNAVAILABLE, "retry"))
                    } else {
                        Ok(response(StatusCode::OK, "ok"))
                    }
                }
            }
        });
        let config = ResiliencyConfig {
            timeout_ms: None,
            retry_policy: Some(super::super::RetryPolicy {
                max_retries: 5,
                retry_on: vec![503],
            }),
        };

        let result = call_with_resiliency(service, (), "/api/flaky", Some(&config))
            .await
            .expect("service should eventually succeed");

        assert_eq!(result.response.status, StatusCode::OK);
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn returns_gateway_timeout_for_slow_service() {
        let service = service_fn(|_: ()| async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            Ok(response(StatusCode::OK, "slow"))
        });
        let config = ResiliencyConfig {
            timeout_ms: Some(10),
            retry_policy: None,
        };

        let error = call_with_resiliency(service, (), "/api/slow", Some(&config))
            .await
            .expect_err("slow service should time out");

        assert_eq!(error.0, StatusCode::GATEWAY_TIMEOUT);
        assert!(error.1.contains("timed out after 10ms"));
    }
}
