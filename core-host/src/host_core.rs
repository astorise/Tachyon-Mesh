#[cfg(feature = "ai-inference")]
pub(crate) use crate::accelerator_component_bindings;
#[cfg(feature = "ai-inference")]
pub(crate) use crate::ai_inference;
#[cfg(test)]
pub(crate) use crate::identity::CallerIdentityClaims;
pub(crate) use crate::identity::HostIdentity;
#[cfg(feature = "websockets")]
pub(crate) use crate::network::handle_websocket_connection;
#[cfg(test)]
pub(crate) use crate::network::{
    handle_tcp_layer4_connection, layer4_bind_address,
    start_udp_layer4_listeners_with_queue_capacity,
};
pub(crate) use crate::network::{
    serve_http_listener, start_http3_listener, start_https_listener, start_mtls_gateway_listener,
    start_tcp_layer4_listeners, start_udp_layer4_listeners, start_uds_fast_path_listener,
};
#[cfg(feature = "rate-limit")]
pub(crate) use crate::rate_limit;
#[cfg(feature = "resiliency")]
pub(crate) use crate::resiliency;
pub(crate) use crate::runtime::{build_command_engine, build_runtime_state};
#[cfg(test)]
pub(crate) use crate::runtime::{
    build_engine, INSTANCE_POOL_DEFAULT_CAPACITY, INSTANCE_POOL_IDLE_TIMEOUT,
};
#[cfg(feature = "http3")]
pub(crate) use crate::server_h3;
pub(crate) use crate::state::{
    AppState, CachedPeerCapabilities, Capabilities, HostLoadCounters, PeerCapabilityCache,
    RuntimeState,
};
#[cfg(feature = "websockets")]
pub(crate) use crate::websocket_component_bindings;
pub(crate) use crate::{
    auth, background_component_bindings, component_bindings, control_plane_component_bindings,
    data_events, memory_governor, network, node_enrollment, store, system_component_bindings,
    system_storage, telemetry, tls_runtime, udp_component_bindings,
};

pub(crate) use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
pub(crate) use anyhow::{anyhow, Context, Result};
pub(crate) use arc_swap::ArcSwap;
#[cfg(feature = "websockets")]
pub(crate) use axum::extract::ws::{Message as AxumWebSocketMessage, WebSocket, WebSocketUpgrade};
pub(crate) use axum::{
    body::{Body, Bytes},
    extract::{Request as AxumRequest, State},
    http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri},
    middleware::from_fn,
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Extension, Router,
};
pub(crate) use clap::{Args as ClapArgs, Parser, Subcommand, ValueEnum};
pub(crate) use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
#[cfg(feature = "websockets")]
pub(crate) use futures_util::{SinkExt, StreamExt};
pub(crate) use http_body_util::BodyExt;
pub(crate) use hyper::body::{Frame, SizeHint};
pub(crate) use hyper::service::service_fn;
pub(crate) use hyper_util::{
    rt::{TokioExecutor, TokioIo},
    server::conn::auto::Builder as HyperConnectionBuilder,
    service::TowerToHyperService,
};
pub(crate) use rand::RngExt;
pub(crate) use reqwest::Client;
pub(crate) use semver::{Version, VersionReq};
pub(crate) use serde::Deserialize;
pub(crate) use serde::Serialize;
pub(crate) use serde_json::{Map, Value};
pub(crate) use sha2::{Digest, Sha256};
#[cfg(unix)]
pub(crate) use std::os::unix::fs::PermissionsExt;
pub(crate) use std::{
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
pub(crate) use telemetry::{TelemetryEvent, TelemetryHandle, TelemetrySnapshot};
#[cfg(unix)]
pub(crate) use tokio::net::UnixListener;
pub(crate) use tokio::sync::Mutex as TokioMutex;
pub(crate) use tokio::sync::{
    mpsc, oneshot, Notify, OwnedSemaphorePermit, Semaphore, TryAcquireError,
};
pub(crate) use tokio_rustls::LazyConfigAcceptor;
pub(crate) use uuid::Uuid;
pub(crate) use wasmtime::{
    component::{Component, Linker as ComponentLinker},
    Config, Engine, Instance, Linker as ModuleLinker, Module, PoolingAllocationConfig,
    ResourceLimiter, Store, Trap, TypedFunc,
};
#[cfg(test)]
pub(crate) use wasmtime_wasi::cli::OutputFile;
pub(crate) use wasmtime_wasi::{
    cli::{InputFile, IsTerminal, StdinStream, StdoutStream},
    p1::{self, WasiP1Ctx},
    p2::{InputStream, OutputStream, Pollable, StreamError, StreamResult},
    DirPerms, FilePerms, I32Exit, ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView,
};
#[cfg(feature = "ai-inference")]
pub(crate) use wasmtime_wasi_nn::witx::WasiNnCtx;

mod app_runtime;
mod background_workers;
mod bridge;
mod component_hosts;
mod config_impls;
mod constants;
mod domain_types;
mod entrypoint;
mod guest_output;
mod guest_runtime;
mod integrity_config;
mod peer_pressure;
mod prewarm;
mod runtime_types;
mod storage_broker;
mod supervisors;
mod uds_fast_path;
mod volumes;

pub(crate) use app_runtime::*;
pub(crate) use bridge::*;
pub(crate) use component_hosts::*;
pub(crate) use constants::*;
pub(crate) use domain_types::*;
pub(crate) use entrypoint::*;
pub(crate) use guest_output::*;
pub(crate) use guest_runtime::*;
pub(crate) use integrity_config::*;
pub(crate) use peer_pressure::*;
pub(crate) use prewarm::*;
pub(crate) use runtime_types::*;
pub(crate) use supervisors::*;
pub(crate) use volumes::*;

#[cfg(test)]
mod tests;
