// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Sentin8:Flow
// SPDX-License-Identifier: Apache-2.0

//! Proxy-back logic for domain-specific steps.
//!
//! When the engine encounters a step kind that is not one of the 7 built-in
//! generic types, it proxy-calls the registering service's internal API.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::types::{ProxyBackRequest, ProxyBackResponse, StepHandlerProfile, StepResult};

/// In-memory registry of step handler profiles, keyed by domain.
/// Lookups fall back to the callback_config provided per-workflow if no
/// registered profile matches the domain.
#[derive(Clone, Default)]
/// The handlerregistry type.
pub struct HandlerRegistry {
    profiles: Arc<RwLock<HashMap<String, StepHandlerProfile>>>,
}

impl HandlerRegistry {
    /// Handle new.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register it.
    pub async fn register(&self, profile: StepHandlerProfile) {
        self.profiles
            .write()
            .await
            .insert(profile.domain.clone(), profile);
    }

    /// Resolve it.
    pub async fn resolve(&self, domain: &str) -> Option<StepHandlerProfile> {
        self.profiles.read().await.get(domain).cloned()
    }
}

/// Proxy-call the domain service's internal API to execute a domain step.
///
/// — the bearer is the engine's M2M token source
/// (`sentin8-flow-sys`, scope `sentin8:sys:steps:execute`) when configured; the
/// static profile `auth_token` is the dev-bootstrap fallback (scope-
/// verifying targets reject it).
/// Handle proxy back.
/// Execute a proxy-back step call. auth DESIGN §4.3f: the
/// `sentin8-flow-sys` CERTIFICATE is the identity — `http` IS the
/// cert-presenting mTLS transport (installed by `build_state` after
/// the workload certificate is ensured); the target service's TLS layer
/// authenticates the handshake. No bearer token exists on this path.
///
/// # Errors
/// Returns an error on transport failure or a non-success status.
pub async fn proxy_back(
    http: &sentin8::middleware::http_client::Sentin8HttpClient,
    profile: &StepHandlerProfile,
    req: ProxyBackRequest,
) -> Result<ProxyBackResponse, anyhow::Error> {
    let url = format!("{}{}/execute", profile.base_url, profile.internal_path);
    let resp = http.post(&url).json(&req).send().await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("proxy-back HTTP {status}: {body}");
    }
    let pb: ProxyBackResponse = resp.json().await?;
    Ok(pb)
}

/// Convert a ProxyBackResponse to a StepResult.
/// Handle proxy back to result.
pub fn proxy_back_to_result(
    pb: ProxyBackResponse,
    step_name: &str,
    kind: &str,
    started: &str,
    ended: &str,
) -> StepResult {
    StepResult {
        step_name: step_name.into(),
        kind: kind.into(),
        status: pb.status,
        output: pb.output,
        error: pb.error,
        next_step: pb.next_step,
        started_at: started.into(),
        ended_at: ended.into(),
    }
}
