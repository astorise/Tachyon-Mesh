#![deny(clippy::unwrap_used)]

#[cfg(test)]
use crate::identity::CallerIdentityClaims;
use crate::identity::HostIdentity;
#[cfg(feature = "websockets")]
use crate::network::handle_websocket_connection;
#[cfg(test)]
use crate::network::{
    handle_tcp_layer4_connection, layer4_bind_address,
    start_udp_layer4_listeners_with_queue_capacity,
};
use crate::network::{
    serve_http_listener, start_http3_listener, start_https_listener, start_mtls_gateway_listener,
    start_tcp_layer4_listeners, start_udp_layer4_listeners, start_uds_fast_path_listener,
};
use crate::runtime::{build_command_engine, build_runtime_state};
#[cfg(test)]
use crate::runtime::{build_engine, INSTANCE_POOL_DEFAULT_CAPACITY, INSTANCE_POOL_IDLE_TIMEOUT};
use crate::state::{
    AppState, CachedPeerCapabilities, Capabilities, HostLoadCounters, PeerCapabilityCache,
    RuntimeState,
};
use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use anyhow::{anyhow, Context, Result};
use arc_swap::ArcSwap;
#[cfg(feature = "websockets")]
use axum::extract::ws::{Message as AxumWebSocketMessage, WebSocket, WebSocketUpgrade};
use axum::{
    body::{Body, Bytes},
    extract::{Request, State},
    http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri},
    middleware::from_fn,
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Extension, Router,
};
use clap::{Args as ClapArgs, Parser, Subcommand, ValueEnum};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
#[cfg(feature = "websockets")]
use futures_util::{SinkExt, StreamExt};
use http_body_util::BodyExt;
use hyper::body::{Frame, SizeHint};
use hyper::service::service_fn;
use hyper_util::{
    rt::{TokioExecutor, TokioIo},
    server::conn::auto::Builder as HyperConnectionBuilder,
    service::TowerToHyperService,
};
use rand::RngExt;
use reqwest::Client;
use semver::{Version, VersionReq};
use serde::Deserialize;
use serde::Serialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
    convert::Infallible,
    fmt, fs,
    hash::{Hash, Hasher},
    io::Write,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, Condvar, Mutex, Once, OnceLock,
    },
    task::{Context as TaskContext, Poll},
    time::{Duration, Instant, SystemTime},
};
use telemetry::{TelemetryEvent, TelemetryHandle, TelemetrySnapshot};
#[cfg(unix)]
use tokio::net::UnixListener;
use tokio::sync::Mutex as TokioMutex;
use tokio::sync::{mpsc, oneshot, Notify, OwnedSemaphorePermit, Semaphore, TryAcquireError};
use tokio_rustls::LazyConfigAcceptor;
use uuid::Uuid;
use wasmtime::{
    component::{Component, Linker as ComponentLinker},
    Config, Engine, Instance, Linker as ModuleLinker, Module, PoolingAllocationConfig,
    ResourceLimiter, Store, Trap, TypedFunc,
};
#[cfg(test)]
use wasmtime_wasi::cli::OutputFile;
use wasmtime_wasi::{
    cli::{InputFile, IsTerminal, StdinStream, StdoutStream},
    p1::{self, WasiP1Ctx},
    p2::{InputStream, OutputStream, Pollable, StreamError, StreamResult},
    DirPerms, FilePerms, I32Exit, ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView,
};
#[cfg(feature = "ai-inference")]
use wasmtime_wasi_nn::witx::WasiNnCtx;

#[cfg(feature = "ai-inference")]
mod ai_inference;
mod auth;
mod core_error;
mod data_events;
mod error;
pub mod identity;
mod memory_governor;
mod mesh;
pub mod network;
mod node_enrollment;
#[cfg(feature = "rate-limit")]
mod rate_limit;
#[cfg(feature = "resiliency")]
mod resiliency;
pub mod runtime;
#[cfg(feature = "http3")]
mod server_h3;
pub mod state;
mod storage;
mod store;
mod system_storage;
pub mod telemetry;
mod tls_runtime;

mod component_bindings {
    wasmtime::component::bindgen!({
        path: "../wit/tachyon.wit",
        world: "faas-guest",
    });
}

mod system_component_bindings {
    wasmtime::component::bindgen!({
        path: "../wit/tachyon.wit",
        world: "system-faas-guest",
    });
}

mod udp_component_bindings {
    wasmtime::component::bindgen!({
        path: "../wit/tachyon.wit",
        world: "udp-faas-guest",
    });
}

#[cfg(feature = "websockets")]
mod websocket_component_bindings {
    wasmtime::component::bindgen!({
        path: "../wit/tachyon.wit",
        world: "websocket-faas-guest",
    });
}

mod background_component_bindings {
    wasmtime::component::bindgen!({
        path: "../wit/tachyon.wit",
        world: "background-system-faas",
    });
}

mod control_plane_component_bindings {
    wasmtime::component::bindgen!({
        path: "../wit/tachyon.wit",
        world: "control-plane-faas",
    });
}

#[cfg(feature = "ai-inference")]
mod accelerator_component_bindings {
    wasmtime::component::bindgen!({
        path: "../wit/accelerator",
        world: "host",
    });
}

include!("host_core.rs");
