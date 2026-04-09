mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../wit",
        world: "faas-guest",
    });

    export!(Component);
}

struct Component;

impl bindings::exports::tachyon::mesh::handler::Guest for Component {
    fn handle_request(
        req: bindings::exports::tachyon::mesh::handler::Request,
    ) -> bindings::exports::tachyon::mesh::handler::Response {
        let payload = if req.body.is_empty() {
            "FaaS received an empty payload".to_owned()
        } else {
            format!("FaaS received: {}", String::from_utf8_lossy(&req.body))
        };
        let env_status = if std::env::var("DB_PASS").is_ok() {
            "present"
        } else {
            "missing"
        };
        let secret_status = match bindings::tachyon::mesh::secrets_vault::get_secret("DB_PASS") {
            Ok(secret) => secret,
            Err(bindings::tachyon::mesh::secrets_vault::Error::NotFound) => "not-found".to_owned(),
            Err(bindings::tachyon::mesh::secrets_vault::Error::PermissionDenied) => {
                "permission-denied".to_owned()
            }
            Err(bindings::tachyon::mesh::secrets_vault::Error::VaultDisabled) => {
                "vault-disabled".to_owned()
            }
        };
        let body = format!("{payload} | env: {env_status} | secret: {secret_status}").into_bytes();

        bindings::exports::tachyon::mesh::handler::Response {
            status: 200,
            headers: vec![],
            body,
            trailers: vec![],
        }
    }
}
