mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../../wit/tachyon.wit",
        world: "background-system-faas",
    });

    export!(Component);
}

const DB_URL_ENV: &str = "DB_URL";
const OUTBOX_TABLE_ENV: &str = "OUTBOX_TABLE";
const TARGET_ROUTE_ENV: &str = "TARGET_ROUTE";
const BATCH_SIZE_ENV: &str = "BATCH_SIZE";
const DEFAULT_BATCH_SIZE: u32 = 16;

struct Component;

impl bindings::Guest for Component {
    fn on_tick() {
        if let Err(error) = poll_once() {
            eprintln!("system-faas-cdc tick failed: {error}");
        }
    }
}

fn poll_once() -> Result<(), String> {
    let db_url = required_env(DB_URL_ENV)?;
    let outbox_table = required_env(OUTBOX_TABLE_ENV)?;
    let target_route = normalize_target_route(&required_env(TARGET_ROUTE_ENV)?)?;
    let batch_size = parse_batch_size();
    let events =
        bindings::tachyon::mesh::outbox_store::claim_events(&db_url, &outbox_table, batch_size)?;

    for event in events {
        let headers = content_type_headers(&event.content_type);
        let response = bindings::tachyon::mesh::outbound_http::send_request(
            "POST",
            &format!("http://mesh{target_route}"),
            &headers,
            &event.body,
        )?;
        if response.status == 200 {
            bindings::tachyon::mesh::outbox_store::ack_event(&db_url, &outbox_table, &event.id)?;
        }
    }

    Ok(())
}

fn required_env(name: &str) -> Result<String, String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("missing required environment variable `{name}`"))
}

fn normalize_target_route(route: &str) -> Result<String, String> {
    let trimmed = route.trim();
    if trimmed.is_empty() {
        return Err("TARGET_ROUTE must not be empty".to_owned());
    }
    Ok(if trimmed.starts_with('/') {
        trimmed.to_owned()
    } else {
        format!("/{trimmed}")
    })
}

fn parse_batch_size() -> u32 {
    std::env::var(BATCH_SIZE_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_BATCH_SIZE)
}

fn content_type_headers(content_type: &str) -> Vec<(String, String)> {
    let trimmed = content_type.trim();
    if trimmed.is_empty() {
        Vec::new()
    } else {
        vec![("content-type".to_owned(), trimmed.to_owned())]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_target_route_adds_leading_slash() {
        assert_eq!(
            normalize_target_route("api/cdc-target").expect("route should normalize"),
            "/api/cdc-target"
        );
        assert_eq!(
            normalize_target_route("/api/cdc-target").expect("route should normalize"),
            "/api/cdc-target"
        );
    }

    #[test]
    fn parse_batch_size_uses_positive_values_only() {
        std::env::set_var(BATCH_SIZE_ENV, "32");
        assert_eq!(parse_batch_size(), 32);

        std::env::set_var(BATCH_SIZE_ENV, "0");
        assert_eq!(parse_batch_size(), DEFAULT_BATCH_SIZE);

        std::env::remove_var(BATCH_SIZE_ENV);
    }

    #[test]
    fn content_type_headers_omits_empty_values() {
        assert!(content_type_headers("").is_empty());
        assert_eq!(
            content_type_headers("application/json"),
            vec![("content-type".to_owned(), "application/json".to_owned())]
        );
    }
}
