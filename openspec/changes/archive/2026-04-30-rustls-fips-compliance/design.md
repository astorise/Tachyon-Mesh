# Design: Conditional Compilation for FIPS

## 1. Cargo.toml Updates
```toml
[features]
default = ["ring"]
ring = ["rustls/ring"]
fips = ["rustls/aws_lc_rs", "rustls/fips"]
```

## 2. TLS Initialization (`core-host/src/tls_runtime.rs`)
```rust
#[cfg(feature = "fips")]
pub fn init_crypto_provider() -> Result<(), CoreError> {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .map_err(|_| CoreError::CryptoInitFailed)?;
    
    tracing::info!("🔒 Tachyon Mesh is running in STRICT FIPS 140-2/3 MODE");
    Ok(())
}

#[cfg(not(feature = "fips"))]
pub fn init_crypto_provider() -> Result<(), CoreError> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| CoreError::CryptoInitFailed)?;
    Ok(())
}
```