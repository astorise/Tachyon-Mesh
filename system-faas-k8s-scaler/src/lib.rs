mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../wit",
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

static mut TICK_COUNT: u64 = 0;
static mut LAST_SCALE_TICK: u64 = 0;

struct Component;

impl bindings::Guest for Component {
    fn on_tick() {
        unsafe {
            TICK_COUNT = TICK_COUNT.saturating_add(1);
        }

        let pending =
            bindings::tachyon::mesh::scaling_metrics::get_pending_queue_size(LEGACY_ROUTE);
        if pending <= SCALE_THRESHOLD {
            return;
        }

        unsafe {
            if LAST_SCALE_TICK != 0 && TICK_COUNT.saturating_sub(LAST_SCALE_TICK) < COOLDOWN_TICKS {
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
                LAST_SCALE_TICK = TICK_COUNT;
            }
        }
    }
}
