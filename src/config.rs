// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Sentin8:Flow
// SPDX-License-Identifier: Apache-2.0

//! Flow configuration loaded from environment variables.

use std::env;

use sentin8::auth::client::{Client as AuthClient, ClientOptions};

/// A step handler profile loaded from `SENTIN8_FLOW_HANDLER_{DOMAIN}_{URL|TOKEN|PATH}`
/// env vars. The domain is the lowercased `{DOMAIN}` segment.
#[derive(Debug, Clone)]
/// Configuration for handlerenv.
pub struct HandlerEnvConfig {
    /// The domain.
    pub domain: String,
    /// The base URL.
    pub base_url: String,
    /// The auth token.
    pub auth_token: String,
    /// The internal path.
    pub internal_path: String,
}

#[derive(Debug, Clone)]
/// Configuration for flow.
pub struct FlowConfig {
    /// The port.
    pub port: u16,
    /// The database URL.
    pub database_url: String,
    /// The sentin8 auth issuer.
    pub sentin8_auth_issuer: String,
    /// The sentin8 auth JWKS URL.
    pub sentin8_auth_jwks_url: String,
    /// The sentin8 bus URL.
    pub sentin8_bus_url: String,
    /// The sentin8 func URL.
    pub sentin8_func_url: String,
    /// Step handler profiles configured via env vars at startup.
    /// The handler profiles.
    pub handler_profiles: Vec<HandlerEnvConfig>,
    /// Shared Auth client - constructed once so the discovery and JWKS
    /// caches are reused across every request (5-minute TTL each).
    /// Cloning is cheap (the caches live behind an `Arc`).
    /// The auth client.
    pub auth_client: AuthClient,
}

impl FlowConfig {
    /// Handle from env.
    pub fn from_env() -> Self {
        let cfg = sentin8::config::Sentin8Config::from_env();
        Self {
            port: env::var("SENTIN8_FLOW_PORT")
                .expect("SENTIN8_FLOW_PORT must be set by captain")
                .parse()
                .expect("SENTIN8_FLOW_PORT must be a valid u16"),
            database_url: env::var("SENTIN8_FLOW_DATABASE_URL").unwrap_or_else(|_| {
                env::var("DATABASE_URL").unwrap_or_else(|_| {
                    sentin8::config::Sentin8Config::default_database_url().to_string()
                })
            }),
            sentin8_auth_issuer: env::var("SENTIN8_AUTH_ISSUER")
                .unwrap_or_else(|_| cfg.sentin8_auth_issuer()),
            sentin8_auth_jwks_url: env::var("SENTIN8_AUTH_JWKS_URL")
                .unwrap_or_else(|_| cfg.sentin8_auth_jwks_url()),
            sentin8_bus_url: env::var("SENTIN8_BUS_URL").unwrap_or_else(|_| cfg.sentin8_bus_url()),
            sentin8_func_url: env::var("SENTIN8_FUNC_URL")
                .unwrap_or_else(|_| cfg.sentin8_func_url()),
            handler_profiles: Self::handler_profiles_from_env(),
            auth_client: AuthClient::new(ClientOptions {
                base_url: env::var("SENTIN8_AUTH_ISSUER")
                    .unwrap_or_else(|_| cfg.sentin8_auth_issuer()),
                ..Default::default()
            }),
        }
    }

    /// Discover `SENTIN8_FLOW_HANDLER_*` env vars. For a domain
    /// `bank8_payments`, the vars are `SENTIN8_FLOW_HANDLER_BANK8_PAYMENTS_URL`,
    /// `..._TOKEN`, `..._PATH`. A domain is included when at least `_URL` is set.
    fn handler_profiles_from_env() -> Vec<HandlerEnvConfig> {
        let mut domains: Vec<String> = env::vars()
            .filter_map(|(k, _)| {
                k.strip_prefix("SENTIN8_FLOW_HANDLER_")
                    .and_then(|rest| rest.strip_suffix("_URL"))
                    .map(|d| d.to_lowercase())
            })
            .collect();
        domains.sort();
        domains.dedup();
        domains
            .into_iter()
            .map(|domain| {
                let prefix = format!("SENTIN8_FLOW_HANDLER_{}", domain.to_uppercase());
                HandlerEnvConfig {
                    domain: domain.clone(),
                    base_url: env::var(format!("{prefix}_URL")).unwrap_or_default(),
                    auth_token: env::var(format!("{prefix}_TOKEN")).unwrap_or_default(),
                    internal_path: env::var(format!("{prefix}_PATH"))
                        .unwrap_or_else(|_| "/v1/internal/steps".into()),
                }
            })
            .collect()
    }
}
