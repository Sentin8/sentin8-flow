// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Sentin8:Flow
// SPDX-License-Identifier: Apache-2.0

//! Sentin8 Flow — standalone workflow execution engine.
//!
//! Boot runs on the platform `sentin8-shell` crate: registration flag,
//! PQ provider, config, capped pool, PG18 check, state binding, OpenAPI
//! merge, middleware stack and PQ-TLS listener are owned by the shell. This
//! binary supplies only its registration data, state and routes.

#[allow(dead_code)]
mod admin_ui;
#[allow(dead_code)]
mod auth;
#[allow(dead_code)]
mod config;
#[allow(dead_code)]
mod engine;
#[allow(dead_code)]
mod func_integration;
#[allow(dead_code)]
mod handlers;
mod openapi;
mod policy;
#[allow(dead_code)]
mod proxy_back;
mod registration;
#[allow(dead_code)]
mod server;
#[allow(dead_code)]
mod steps_generic;
#[allow(dead_code)]
mod store;
#[allow(dead_code)]
mod types;

use utoipa::OpenApi;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    sentin8::shell::serve(
        sentin8::shell::Sentin8Shell::new("sentin8:flow", "SENTIN8_FLOW")
            .with_registration(registration::print)
            .with_state(server::build_state)
            .with_own_health()
            .with_router(|router, _state| server::routes(router))
            .with_openapi(openapi::FlowOpenApi::openapi()),
    )
    .await
}
