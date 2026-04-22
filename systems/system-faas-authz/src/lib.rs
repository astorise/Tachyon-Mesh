mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../../wit/authz.wit",
        world: "authz-guest",
    });

    export!(Component);
}

use bindings::exports::tachyon::identity::authz::IdentityPayload;

struct Component;

impl bindings::exports::tachyon::identity::authz::Guest for Component {
    fn evaluate_policy(
        identity: IdentityPayload,
        action: String,
        resource: String,
    ) -> Result<bool, bindings::exports::tachyon::identity::authz::AuthzError> {
        let roles = normalize(identity.roles);
        if roles.iter().any(|role| role == "admin") {
            return Ok(true);
        }

        let scopes = normalize(identity.scopes);
        let required = required_scopes(&action, &resource);
        if required.is_empty() {
            return Ok(false);
        }

        Ok(required
            .iter()
            .all(|scope| scopes.iter().any(|owned| owned == scope)))
    }
}

fn normalize(values: Vec<String>) -> Vec<String> {
    let mut values = values
        .into_iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    values.sort();
    values.dedup();
    values
}

fn required_scopes(action: &str, resource: &str) -> Vec<&'static str> {
    let _action = action.trim().to_ascii_uppercase();
    let resource = resource.trim();

    if resource == "/admin/status" {
        return vec!["read:nodes"];
    }
    if resource == "/admin/assets" {
        return vec!["deploy:wasm"];
    }
    if resource.starts_with("/admin/models/") {
        return vec!["deploy:models"];
    }
    if resource == "/admin/security/pats" {
        return vec!["manage:tokens"];
    }
    if resource == "/admin/security/recovery-codes" || resource == "/admin/security/2fa/regenerate"
    {
        return vec!["manage:security"];
    }

    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bindings::exports::tachyon::identity::authz::Guest;

    fn identity(roles: &[&str], scopes: &[&str]) -> IdentityPayload {
        IdentityPayload {
            subject: "admin".to_owned(),
            roles: roles.iter().map(|value| (*value).to_owned()).collect(),
            scopes: scopes.iter().map(|value| (*value).to_owned()).collect(),
        }
    }

    #[test]
    fn admin_role_bypasses_scope_checks() {
        assert!(Component::evaluate_policy(
            identity(&["admin"], &[]),
            "POST".to_owned(),
            "/admin/security/pats".to_owned()
        )
        .expect("policy should evaluate"));
    }

    #[test]
    fn scoped_pat_can_upload_assets() {
        assert!(Component::evaluate_policy(
            identity(&[], &["deploy:wasm"]),
            "POST".to_owned(),
            "/admin/assets".to_owned()
        )
        .expect("policy should evaluate"));
    }

    #[test]
    fn policy_denies_unknown_admin_routes_without_admin_role() {
        assert!(!Component::evaluate_policy(
            identity(&[], &["read:nodes"]),
            "GET".to_owned(),
            "/admin/unknown".to_owned()
        )
        .expect("policy should evaluate"));
    }
}
