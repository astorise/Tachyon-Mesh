mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../wit",
        world: "system-faas-guest",
    });

    export!(Component);
}

use bindings::tachyon::mesh::storage_broker::WriteMode;
#[cfg(not(target_arch = "wasm32"))]
use rcgen::generate_simple_self_signed;
use serde::{Deserialize, Serialize};

const ACME_STAGING_MOCK: &str = "ACME_STAGING_MOCK";
#[cfg(target_arch = "wasm32")]
const MOCK_CERTIFICATE_PEM: &str = "-----BEGIN CERTIFICATE-----\n\
MIIC1zCCAb+gAwIBAgIIL4ARv71JyR0wDQYJKoZIhvcNAQELBQAwGzEZMBcGA1UEAxMQYXBpLmV4\n\
YW1wbGUudGVzdDAeFw0yNjA0MDgyMDAwMTdaFw0yNzA0MDgyMDAxMTdaMBsxGTAXBgNVBAMTEGFw\n\
aS5leGFtcGxlLnRlc3QwggEiMA0GCSqGSIb3DQEBAQUAA4IBDwAwggEKAoIBAQDrmjAl5zo9zWP5\n\
m3PFTXkU3l57tLA5qhABbWSCAYZHod00ItVfzljcn0mA/oeUC4tMQ/zuv1z3qaxKAZ35q06hSEkw\n\
ERYHJrnAKSOU6c9R2bgwwsP5nIGKM0hgIwFOMbIMckX5XwTg9iXxNAnzRoyI4II46hDOqHGxwpcV\n\
sGqfSQiay450dwmmk4mIX/MtKeA9Zaav4Igl1bgjUV1D8rtwyQyhxSeYknE97dPls6YzxbkW+Bay\n\
I6a4MaoH6fXvhvKNESguEiyr62eggfcJD6NWOFcAES1u+uNy8xU+vK7atN+HseRtdFqf/Ik/0eQS\n\
9QMG/wDnWnv3r+4RnCudpTDJAgMBAAGjHzAdMBsGA1UdEQQUMBKCEGFwaS5leGFtcGxlLnRlc3Qw\n\
DQYJKoZIhvcNAQELBQADggEBAI/jP+bMfScWfZUCVIH/4zPOn2yvdVJWIlsJ5AhJ6Fzcdon0pttN\n\
AJQlMBGuz+Pserc9Q8o7VFhx9CXxUbhHLeUHY/E0H7J8cpzw58L8MHyYwzH2Qwlly52SeQgTKKN7\n\
I257n9ynLo0lTAxDj2U9S3cH2BCLZE1Caac9DC8C3ZKdoKLxJx3Oqa4WCly7gmDFZuVA3ZOlEUOp\n\
D5wk8mb3G2eUsIgoph6Lr39JJnS68JW0ldjpUzVcwODIkK5YlOYmEKKdk98KQ7ekbG51rCzW/cRH\n\
zyB8leXXFu4ICWsDUW0AlIyxSmHl7avn69xrjqNMD4zyI5s+8NjbOPItKn2DQk0=\n\
-----END CERTIFICATE-----\n";
#[cfg(target_arch = "wasm32")]
const MOCK_PRIVATE_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----\n\
MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQDrmjAl5zo9zWP5m3PFTXkU3l57\n\
tLA5qhABbWSCAYZHod00ItVfzljcn0mA/oeUC4tMQ/zuv1z3qaxKAZ35q06hSEkwERYHJrnAKSOU\n\
6c9R2bgwwsP5nIGKM0hgIwFOMbIMckX5XwTg9iXxNAnzRoyI4II46hDOqHGxwpcVsGqfSQiay450\n\
dwmmk4mIX/MtKeA9Zaav4Igl1bgjUV1D8rtwyQyhxSeYknE97dPls6YzxbkW+BayI6a4MaoH6fXv\n\
hvKNESguEiyr62eggfcJD6NWOFcAES1u+uNy8xU+vK7atN+HseRtdFqf/Ik/0eQS9QMG/wDnWnv3\n\
r+4RnCudpTDJAgMBAAECggEAKCbp57vFeDzlueddTpXKedz/2zNLCTjLa4LaKzHZUaHrUfRRyvce\n\
u9LFsx8tufRRtBiuJX4leOvIugAWjTM9vkzUdEWlLGjUJUSdlMZYF8n0ExNOVN7wUL42qnOsyEe9\n\
4VMkS8B+01v/0WCeBYDTeIxShSKW5LFeVv4jw4WCVkzHUNJIrS4p2asFntqygpxtq0ee7EBnty4N\n\
oyAFlV9fGcXIz3WmmE792+3X5CBE4JhnZwsfLWZ4PvqIIGrJqNheF32owRiqnfciF7uUORCXAtzN\n\
jqv8HLTUeKvE3PRujiu9qivmUM9ef9kPfIJXWipaUw1QcLk3mbfnZFsh6UY2FQKBgQDtHZeguCX/\n\
/4XygBoWw24oHPwS+1zLBQlNBFPEJo+7oIspxBHtp+KbarvXsQSzFdkCzbHtJcEHK6ufqKZPSU47\n\
t1g0E+FhokizL6mkfEQZXKcBZ0boS20UAEVY7gEzqSN0zH0HB0zK68qF948uchhonk7SB6wjlF0n\n\
Tuku/+YftwKBgQD+Xb33yX/i/UiDHf6L9Gd6zD3LRH6Dfgt0SQMJdmNshcCif9pIIQNO5LDDVyza\n\
NJBRuS7qb6DE4FOLR1ZOe3N76MZyuKGF6uegLpyvImbqdc0xfGzILtULHyaJqs2w/1G/T2vFegLQ\n\
A5ozvwn79cQud3UVIhEgJHDMwoD3yyYzfwKBgQDhKkrEqlobgXCXWaJsn2TJ3sxY0i3J9JxicIuD\n\
JwMyrz+3h6NmxRhhcbezGTxXO5X6HY6qnkFxJ70wPhzACeKqvm6Z9Y7/AfZ7gfVcZ0zbsKo+oO4q\n\
xQVuCtvPmSO3BRTQYycPN5Vq1QJauT1UY7BeGIbM19BVcRwMqdixcvv6fQKBgEHbc3vcJ8hVW5jX\n\
AzipJsGcb8NZEIhq8fxBiw/AHy3R03Y/M/zIz1p1y25H+8zjHxqJn6QDEtTmX7sH1UisndHPCtJZ\n\
CzjpAN9wMhEGDy9VILNXS7LorTAb+JZcKrVQ5ZFqtrSCSogg5qPPKn6ZuxlsxFucXmK8DJh3I30E\n\
k/dxAoGAUIybvptmZTvJUwPvl6i9cNuA80oH4GRMrfboYW6YYja++CpX71k/Vx5UgzKlYeq3BIMd\n\
JRLnIUlWtlxC66AtuNcqbJNbC5OWYCPGdLHbVd2aHCG/tSqINerY527QPW758Dl//Qa72wTQx3vV\n\
pt79cED8u9mUVLgqKjlduogu588=\n\
-----END PRIVATE KEY-----\n";

struct Component;

#[derive(Debug, PartialEq, Eq)]
struct CertificateRequest {
    domain: String,
    storage_path: String,
    mode: String,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct CertificateBundle {
    domain: String,
    certificate_pem: String,
    private_key_pem: String,
}

impl bindings::exports::tachyon::mesh::handler::Guest for Component {
    fn handle_request(
        req: bindings::exports::tachyon::mesh::handler::Request,
    ) -> bindings::exports::tachyon::mesh::handler::Response {
        if !req.method.eq_ignore_ascii_case("POST") {
            return response(405, "Method Not Allowed");
        }

        let request = match parse_certificate_request(&req.uri) {
            Ok(request) => request,
            Err(message) => return response(400, message),
        };

        if request.mode != ACME_STAGING_MOCK {
            return response(400, "cert-manager `mode` must be `ACME_STAGING_MOCK`");
        }

        let bundle = match issue_self_signed_certificate(&request.domain) {
            Ok(bundle) => bundle,
            Err(message) => return response(500, message),
        };

        let body = match serde_json::to_vec(&bundle) {
            Ok(body) => body,
            Err(error) => {
                return response(
                    500,
                    format!("failed to serialize certificate bundle: {error}"),
                );
            }
        };

        if let Err(error) = bindings::tachyon::mesh::storage_broker::enqueue_write(
            &request.storage_path,
            WriteMode::Overwrite,
            &body,
        ) {
            let (status, body) = map_storage_error(error);
            return response(status, body);
        }

        response(200, body)
    }
}

fn parse_certificate_request(uri: &str) -> Result<CertificateRequest, &'static str> {
    let query = uri
        .split_once('?')
        .map(|(_, query)| query)
        .ok_or("cert-manager requests must include query parameters")?;
    let mut domain = None;
    let mut storage_path = None;
    let mut mode = None;

    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        match key {
            "domain" => domain = Some(value.to_owned()),
            "storage_path" => storage_path = Some(value.to_owned()),
            "mode" => mode = Some(value.to_owned()),
            _ => {}
        }
    }

    let domain = require_query_value(domain, "domain")?;
    if domain.contains('/') || domain.contains('\\') || domain.contains(' ') {
        return Err("cert-manager `domain` must be a single hostname");
    }

    Ok(CertificateRequest {
        domain,
        storage_path: require_query_value(storage_path, "storage_path")?,
        mode: require_query_value(mode, "mode")?,
    })
}

fn require_query_value(value: Option<String>, key: &'static str) -> Result<String, &'static str> {
    let value = value.ok_or(match key {
        "domain" => "cert-manager requests must include `domain`",
        "storage_path" => "cert-manager requests must include `storage_path`",
        "mode" => "cert-manager requests must include `mode`",
        _ => "cert-manager request is missing a required query parameter",
    })?;

    if value.trim().is_empty() {
        Err(match key {
            "domain" => "cert-manager `domain` must not be empty",
            "storage_path" => "cert-manager `storage_path` must not be empty",
            "mode" => "cert-manager `mode` must not be empty",
            _ => "cert-manager query parameter must not be empty",
        })
    } else {
        Ok(value)
    }
}

fn issue_self_signed_certificate(domain: &str) -> Result<CertificateBundle, String> {
    #[cfg(target_arch = "wasm32")]
    {
        return Ok(mock_certificate_bundle(domain));
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        let certified = generate_simple_self_signed(vec![domain.to_owned()])
            .map_err(|error| format!("failed to issue self-signed certificate: {error}"))?;

        Ok(CertificateBundle {
            domain: domain.to_owned(),
            certificate_pem: certified.cert.pem(),
            private_key_pem: certified.key_pair.serialize_pem(),
        })
    }
}

#[cfg(target_arch = "wasm32")]
fn mock_certificate_bundle(domain: &str) -> CertificateBundle {
    CertificateBundle {
        domain: domain.to_owned(),
        certificate_pem: MOCK_CERTIFICATE_PEM.to_owned(),
        private_key_pem: MOCK_PRIVATE_KEY_PEM.to_owned(),
    }
}

fn map_storage_error(error: String) -> (u16, Vec<u8>) {
    if let Some(message) = error.strip_prefix("forbidden:") {
        return (403, message.trim().as_bytes().to_vec());
    }

    (500, error.into_bytes())
}

fn response(
    status: u16,
    body: impl Into<Vec<u8>>,
) -> bindings::exports::tachyon::mesh::handler::Response {
    bindings::exports::tachyon::mesh::handler::Response {
        status,
        headers: vec![],
        body: body.into(),
        trailers: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_certificate_request_extracts_expected_fields() {
        let request = parse_certificate_request(
            "/system/cert-manager?domain=api.example.test&storage_path=/app/certs/api.example.test.json&mode=ACME_STAGING_MOCK",
        )
        .expect("request should parse");

        assert_eq!(
            request,
            CertificateRequest {
                domain: "api.example.test".to_owned(),
                storage_path: "/app/certs/api.example.test.json".to_owned(),
                mode: ACME_STAGING_MOCK.to_owned(),
            }
        );
    }

    #[test]
    fn parse_certificate_request_rejects_invalid_domain() {
        let error = parse_certificate_request(
            "/system/cert-manager?domain=api/example.test&storage_path=/app/certs/api.example.test.json&mode=ACME_STAGING_MOCK",
        )
        .expect_err("invalid domain should fail");

        assert_eq!(error, "cert-manager `domain` must be a single hostname");
    }

    #[test]
    fn self_signed_certificate_contains_domain_and_pem_material() {
        let bundle =
            issue_self_signed_certificate("api.example.test").expect("bundle should be issued");

        assert_eq!(bundle.domain, "api.example.test");
        assert!(bundle.certificate_pem.contains("BEGIN CERTIFICATE"));
        assert!(bundle.private_key_pem.contains("BEGIN PRIVATE KEY"));
    }
}
