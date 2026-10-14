// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Sentin8:Flow
// SPDX-License-Identifier: Apache-2.0

//! Auth token verification for Flow.
//!
//! Re-exports the shared types from the sentin8-auth-client SDK and provides
//! thin wrappers matching the original Flow call sites (taking `&FlowConfig` + `&HeaderMap`).

use axum::http::{HeaderMap, StatusCode};

use crate::config::FlowConfig;

pub use sentin8::auth::client::AuthContext;

/// Verify a Bearer token. Returns `None` if no token is present;
/// returns `Err` if a token is present but invalid.
pub async fn verify_token_opt(
    config: &FlowConfig,
    headers: &HeaderMap,
) -> Result<Option<AuthContext>, (StatusCode, String)> {
    sentin8::auth::client::verify_bearer_token_opt(&config.auth_client, headers).await
}

/// Require a token to be present and valid.
pub async fn require_token(
    config: &FlowConfig,
    headers: &HeaderMap,
) -> Result<AuthContext, (StatusCode, String)> {
    verify_token_opt(config, headers).await?.ok_or((
        StatusCode::UNAUTHORIZED,
        "missing Authorization header".into(),
    ))
}

/// Require any of the given scopes (platform admins bypass).
pub async fn require_any_scope(
    config: &FlowConfig,
    headers: &HeaderMap,
    scopes: &[&str],
) -> Result<AuthContext, (StatusCode, String)> {
    let a = require_token(config, headers).await?;
    a.require_any_scope(scopes)?;
    Ok(a)
}

/// Certificate-first lifecycle authentication (auth DESIGN §4.3f; group
/// DESIGN.md §3.1): an enrolled platform workload presents its Acme
/// client certificate in the TLS handshake — the TLS layer (having
/// CA-verified it) injects `X-Sentin8-Client-Cert-Subject/-Thumbprint`
/// and strips client-supplied copies. The subject resolves through the
/// boot-loaded registry map to scopes; the certificate IS the identity
/// (no bearer, no DCR — the retired token model). Bearer tokens remain
/// the fallback for user/delegated callers.
///
/// Mirrors Bus's combined verifier — the same trust model every
/// cert-facing service uses.
pub async fn verify_cert_or_token(
    registry: &sentin8::auth::workload_registry::WorkloadMap,
    config: &FlowConfig,
    headers: &HeaderMap,
) -> Result<AuthContext, (StatusCode, String)> {
    // The TLS layer injects these ONLY after CA-verifying the presented
    // certificate (client-supplied copies are stripped before injection).
    let subject = headers
        .get("x-sentin8-client-cert-subject")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty());
    let Some(subject) = subject else {
        return require_token(config, headers).await;
    };
    let Some(workload) = subject.strip_prefix("m2m-").filter(|w| !w.is_empty()) else {
        return Err((
            StatusCode::UNAUTHORIZED,
            format!("certificate CN '{subject}' is not a platform workload identity"),
        ));
    };
    let Some(rec) = registry.get(workload).await else {
        return Err((
            StatusCode::UNAUTHORIZED,
            format!("workload '{workload}' is not enrolled"),
        ));
    };
    if !rec.enabled {
        return Err((
            StatusCode::UNAUTHORIZED,
            format!("workload '{workload}' is disabled"),
        ));
    }
    let now = chrono::Utc::now().timestamp();
    // Synthetic request-scoped claims: the identity is the certificate
    // (valid until Acme-side expiry or registry revocation, observed
    // within the map's TTL). Nothing caches these.
    let (scope_str, vertical) = (rec.scopes.join(" "), rec.vertical.clone());
    Ok(AuthContext {
        claims: sentin8::auth::client::VerifiedClaims {
            sub: format!("m2m-{workload}"),
            iss: config.sentin8_auth_issuer.clone(),
            aud: String::new(),
            jti: "cert".to_string(),
            iat: now,
            exp: i64::MAX,
            token_tp: "m2m".to_string(),
            scope: scope_str.clone(),
            env_id: Some(sentin8::config::SYSTEM_ENV_ID.to_string()),
            email: None,
            act: None,
            vertical: Some(vertical),
            service_type: Some(rec.service_type.clone()),
        },
        workload: Some(sentin8::auth::client::WorkloadIdentity {
            workload: workload.to_string(),
            scopes: rec.scopes.clone(),
            vertical: Some(rec.vertical),
            // VDC-4: the service_type claim rides the identity so Sys's
            // tenant-snapshot scoped discovery keys on it.
            service_type: Some(rec.service_type.clone()),
            thumbprint: headers
                .get("x-sentin8-client-cert-thumbprint")
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_string(),
        }),
    })
}
