use std::sync::atomic::{AtomicU64, Ordering};

mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../../wit/tachyon.wit",
        world: "background-system-faas",
    });

    export!(Component);
}

const LEGACY_ROUTE: &str = "/api/guest-call-legacy";
const KUBERNETES_DEPLOYMENT_URL: &str =
    "https://kubernetes.default.svc/apis/apps/v1/namespaces/default/deployments/legacy-app";
const SCALE_THRESHOLD: u32 = 50;
const COOLDOWN_TICKS: u64 = 6;
const DESIRED_REPLICAS: u32 = 2;

// This guest is single-threaded WASM, so a `static mut` counter here was
// always sound in practice, but it is a hard error under the 2024 edition
// (`static_mut_refs`) and required `unsafe` for no benefit over a relaxed
// atomic, which needs none.
static TICK_COUNT: AtomicU64 = AtomicU64::new(0);
static LAST_SCALE_TICK: AtomicU64 = AtomicU64::new(0);

struct Component;

impl bindings::Guest for Component {
    fn on_tick() {
        let tick_count = TICK_COUNT.fetch_add(1, Ordering::Relaxed).saturating_add(1);

        let pending =
            bindings::tachyon::mesh::scaling_metrics::get_pending_queue_size(LEGACY_ROUTE);
        if pending <= SCALE_THRESHOLD {
            return;
        }

        let last_scale_tick = LAST_SCALE_TICK.load(Ordering::Relaxed);
        if last_scale_tick != 0 && tick_count.saturating_sub(last_scale_tick) < COOLDOWN_TICKS {
            return;
        }

        let body = format!(r#"{{"spec":{{"replicas":{DESIRED_REPLICAS}}}}}"#).into_bytes();
        let headers = vec![(
            "content-type".to_owned(),
            "application/merge-patch+json".to_owned(),
        )];
        if bindings::tachyon::mesh::outbound_http::send_request(
            "PATCH",
            KUBERNETES_DEPLOYMENT_URL,
            &headers,
            &body,
        )
        .is_ok()
        {
            LAST_SCALE_TICK.store(tick_count, Ordering::Relaxed);
        }
    }
}
