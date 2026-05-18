use super::super::*;
use crate::store::secrets::{clear_secrets, register_secret, test_registry_guard};

#[test]
fn outbound_secret_interceptor_rewrites_allowed_headers_and_body() {
    let _guard = test_registry_guard();
    clear_secrets().expect("registry should clear");
    let id =
        Uuid::parse_str("550e8400-e29b-41d4-a716-446655440100").expect("static UUID should parse");
    register_secret(id, "sk-live-openai", vec!["api.openai.com".to_owned()])
        .expect("secret should register");

    let (headers, body) = inject_outbound_secrets(
        vec![(
            "authorization".to_owned(),
            "Bearer tachyon:secret:550e8400-e29b-41d4-a716-446655440100".to_owned(),
        )],
        b"{\"api_key\":\"tachyon:secret:550e8400-e29b-41d4-a716-446655440100\"}".to_vec(),
        Some("api.openai.com"),
    );

    assert_eq!(
        headers,
        vec![(
            "authorization".to_owned(),
            "Bearer sk-live-openai".to_owned()
        )]
    );
    assert_eq!(body, b"{\"api_key\":\"sk-live-openai\"}");
}

#[test]
fn outbound_secret_interceptor_keeps_placeholder_for_disallowed_host() {
    let _guard = test_registry_guard();
    clear_secrets().expect("registry should clear");
    let id =
        Uuid::parse_str("550e8400-e29b-41d4-a716-446655440101").expect("static UUID should parse");
    register_secret(id, "sk-live-openai", vec!["api.openai.com".to_owned()])
        .expect("secret should register");

    let placeholder = "tachyon:secret:550e8400-e29b-41d4-a716-446655440101";
    let (headers, body) = inject_outbound_secrets(
        vec![("authorization".to_owned(), format!("Bearer {placeholder}"))],
        placeholder.as_bytes().to_vec(),
        Some("evil.test"),
    );

    assert_eq!(
        headers,
        vec![("authorization".to_owned(), format!("Bearer {placeholder}"))]
    );
    assert_eq!(body, placeholder.as_bytes());
}
