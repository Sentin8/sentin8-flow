// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Sentin8:Flow
// SPDX-License-Identifier: Apache-2.0

//! Axum server setup for Flow.

use std::sync::Arc;

use axum::routing::{get, post};

use crate::engine::Engine;
use crate::handlers::{self, AppState};
use crate::proxy_back::HandlerRegistry;

/// Build state.
pub async fn build_state(
    core: sentin8::shell::ServiceCore,
) -> anyhow::Result<crate::handlers::AppState> {
    let config = crate::config::FlowConfig::from_env();
    let registry = HandlerRegistry::new();

    // Load env-var-configured step handler profiles.
    for p in &config.handler_profiles {
        registry
            .register(crate::types::StepHandlerProfile {
                domain: p.domain.clone(),
                base_url: p.base_url.clone(),
                auth_token: p.auth_token.clone(),
                internal_path: p.internal_path.clone(),
            })
            .await;
    }

    // - auth DESIGN §4.3f: the shared identity material for the
    // `sentin8-flow-proxy-back` workload CERTIFICATE (load-or-mint from
    // Acme - the certificate IS the identity; no DCR, no token). Both
    // the proxy-back transport and the WASM-execution Func lane present
    // it; scope-verifying targets authenticate the handshake; the
    // registry map authorizes the scope.
    let auth_issuer = config.sentin8_auth_issuer.clone();
    let ca_cert_path = sentin8::auth::m2m::platform_ca_path();

    // auth DESIGN §4.3f: the WASM-execution lane presents the SAME
    // `sentin8-flow-proxy-back` workload certificate the proxy-back
    // transport mints - Func's cert-first gate resolves it against the
    // registry map (the static SENTIN8_FUNC_TOKEN dev convention is
    // retired). Boot-retry like every mint consumer (Acme may still be
    // coming up at first birth).
    let func_client = if !config.sentin8_func_url.is_empty() {
        loop {
            match sentin8::auth::m2m::ensure_workload_certificate(
                &auth_issuer,
                "sentin8-flow-proxy-back",
                &ca_cert_path,
            )
            .await
            {
                Ok(cert) => {
                    match sentin8_func_sdk::client::FuncClient::with_workload_identity(
                        &config.sentin8_func_url,
                        &cert.ca_cert_path,
                        &cert.cert_path,
                        &cert.key_path,
                    ) {
                        Ok(c) => break Some(c),
                        Err(e) => {
                            tracing::debug!(error = %e, "flow: func transport build failed - retrying");
                            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                        }
                    }
                }
                Err(e) => {
                    tracing::debug!(error = %e, "flow: func certificate not ready yet - retrying");
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
            }
        }
    } else {
        None
    };

    // - auth DESIGN §4.3f: build the cert-presenting transport for
    // proxy-back calls from the same workload certificate minted above.
    let proxy_back_http = sentin8::auth::m2m::ensure_workload_certificate(
        &auth_issuer,
        "sentin8-flow-proxy-back",
        &ca_cert_path,
    )
    .await
    .and_then(|cert| {
        let mtls = sentin8::middleware::mtls::MtlsConfig::new(
            &cert.ca_cert_path,
            &cert.cert_path,
            &cert.key_path,
        );
        sentin8::middleware::mtls::build_mtls_client(&mtls)
            .map(sentin8::middleware::http_client::Sentin8HttpClient::from_client)
            .map_err(sentin8::auth::m2m::M2mError::Cert)
    })
    .map_err(|e| {
        tracing::warn!(error = %e, "flow: proxy-back transport build failed (proxy-back will fail until next boot)");
        e
    })
    .ok()
    .unwrap_or_else(|| {
        // Fail-soft dev fallback: a plain transport (targets reject it
        // loudly) rather than aborting boot - the certificate mint can
        // succeed on the next restart once Acme is up.
        tracing::warn!("flow: proxy-back falling back to a plain transport (no identity)");
        sentin8::middleware::http_client::Sentin8HttpClient::new()
    });

    let engine = Arc::new(Engine {
        pool: core.pool.clone(),
        http: proxy_back_http,
        registry: registry.clone(),
        func_client,
    });

    // ── Sentin8:Policy (- shell-owned boot): consume the
    // engine the shell booted in step 8 - no second engine, consumer, or
    // DCR registration here.
    let policy_engine = core.admin.policy.clone();

    // auth DESIGN §4.3f: the boot-loaded workload registry map - the
    // authorization record for certificate-only platform-workload calls
    // (sys's delegation presents the `sys-flow` certificate; scopes ride
    // the map, not a token). Unavailable at boot → fail-closed until
    // the next boot (same posture as bus).
    let workload_registry = match sentin8::auth::workload_registry::WorkloadMap::boot_load(
        "sentin8:flow",
    )
    .await
    {
        Ok(map) => map,
        Err(e) => {
            tracing::warn!(error = %e, "flow: workload registry map unavailable at boot - certificate-workload calls fail-closed until next boot");
            sentin8::auth::workload_registry::WorkloadMap::boot_load_empty("sentin8:flow")
        }
    };

    let state = AppState {
        config: Arc::new(config.clone()),
        pool: core.pool.clone(),
        engine,
        registry,
        policy: policy_engine.clone(),
        workload_registry,
        admin: sentin8::shield::admin::SharedAdminState {
            service: "sentin8:flow".to_string(),
            pool: core.pool.clone(),
            policy: policy_engine.clone(),
            auth_client: Arc::new(config.auth_client.clone()),
        },
    };

    // The shell-spawned outbox relay (shell boot step 8b) drains flow's
    // LOCAL `flow.outbox` and pushes rows to Bus `POST /v1/produce`.
    // Flow handlers INSERT into `flow.outbox` via `store::insert_outbox`.

    Ok(state)
}

/// Add this service's routes onto the shell-provided router. The shell
/// owns the boot sequence, state binding, OpenAPI merge and the standard
/// middleware stack; this adds ONLY routes (including the service's own
/// `/v1/health` - custom payload).
/// Handle routes.
pub fn routes(
    router: axum::Router<crate::handlers::AppState>,
) -> axum::Router<crate::handlers::AppState> {
    router
        .route("/v1/health", get(handlers::health))
        // ── Admin UI (RFW) ─────────────────────────────────────────────────
        .route("/v1/admin/ui/{*page}", get(crate::admin_ui::get_admin_ui))
        .route("/v1/admin/{resource}", get(crate::admin_ui::get_admin_data))
        // Workflow API
        .route(
            "/v1/workflows",
            post(handlers::create_workflow).get(handlers::list_workflows),
        )
        .route("/v1/workflows/{id}", get(handlers::get_workflow))
        .route("/v1/workflows/{id}/retry", post(handlers::retry_workflow))
        .route("/v1/workflows/{id}/repair", post(handlers::repair_workflow))
        .route("/v1/workflows/{id}/cancel", post(handlers::cancel_workflow))
        .route("/v1/workflows/{id}/resume", post(handlers::resume_workflow))
        // Handler registration
        .route("/v1/handlers/register", post(handlers::register_handler))
        // Internal API (proxy-back from Cron / Func / external)
        .route(
            "/v1/internal/schedules/run",
            post(handlers::internal_schedule_run),
        )
        .route(
            "/v1/internal/callbacks/async",
            post(handlers::internal_async_callback),
        )
        // Traffic counter - counts every inbound request (excludes health checks).
        // ── Sentin8:Policy admin API ( - shared implementation,
        // one surface per service; handlers are generic over this AppState).
        .route(
            "/v1/admin/policy-sets",
            get(sentin8::shield::admin::list_policy_sets::<handlers::AppState>),
        )
        .route(
            "/v1/admin/policy-sets/{name}",
            get(sentin8::shield::admin::get_policy_set::<handlers::AppState>)
                .put(sentin8::shield::admin::put_policy_set::<handlers::AppState>)
                .delete(sentin8::shield::admin::delete_policy_set::<handlers::AppState>),
        )
        .route(
            "/v1/admin/policy-sets/{name}/test",
            post(sentin8::shield::admin::test_policy_set::<handlers::AppState>),
        )
        .route(
            "/v1/admin/policy-sets/enforce",
            post(sentin8::shield::admin::admin_enforce_policy_set::<handlers::AppState>),
        )
        .route(
            "/v1/admin/policy-sets/shadow",
            post(sentin8::shield::admin::admin_shadow_policy_set::<handlers::AppState>),
        )
        .route(
            "/v1/admin/policy-sets/disable",
            post(sentin8::shield::admin::admin_disable_policy_set::<handlers::AppState>),
        )
        .route(
            "/v1/admin/relations",
            get(sentin8::shield::admin::list_relations::<handlers::AppState>)
                .put(sentin8::shield::admin::put_relation::<handlers::AppState>)
                .delete(sentin8::shield::admin::delete_relation::<handlers::AppState>),
        )
        .route(
            "/v1/admin/relations/check",
            get(sentin8::shield::admin::check_relation::<handlers::AppState>),
        )
        .route(
            "/v1/admin/attributes",
            get(sentin8::shield::admin::list_attributes::<handlers::AppState>)
                .put(sentin8::shield::admin::put_attribute::<handlers::AppState>)
                .delete(sentin8::shield::admin::delete_attribute::<handlers::AppState>),
        )
}

/// Shared policy-admin surface accessor - the shared
/// handlers extract this service's admin state via [`HasPolicyAdmin`].
impl sentin8::shield::admin::HasPolicyAdmin for AppState {
    fn policy_admin(&self) -> &sentin8::shield::admin::SharedAdminState {
        &self.admin
    }
}
