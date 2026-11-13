// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Sentin8:Flow
// SPDX-License-Identifier: Apache-2.0

//! Sentin8 Flow - library re-exports for integration tests.

#![deny(missing_docs)]

/// Admin UI (RFW) handlers.
pub mod admin_ui;
/// Produce/consume authentication helpers.
pub mod auth;
/// Environment configuration.
pub mod config;
/// The workflow state machine engine.
pub mod engine;
/// Sentin8:Func host-function integration for steps.
pub mod func_integration;
/// HTTP handlers - workflow CRUD, lifecycle verbs.
pub mod handlers;
/// OpenAPI spec assembly.
pub mod openapi;

// The utoipa dump for registration::print - mirrored from main.rs; the
// lib crate compiles registration::print too (test harnesses embedding
// via #[path] never invoke it, so the pointer is referenced, not run).
static OPENAPI_DUMP: fn() -> serde_json::Value = || {
    use utoipa::OpenApi as _;
    serde_json::to_value(openapi::FlowOpenApi::openapi()).unwrap_or(serde_json::Value::Null)
};

/// Shield (Sentin8:Policy) integration - shared bridge re-exports.
pub mod policy;
/// Proxy-back callback execution for domain steps.
pub mod proxy_back;
/// Service self-registration declarations.
pub mod registration;
/// Axum router and server.
pub mod server;
/// Generic step executors (HTTP, manual decision, etc.).
pub mod steps_generic;
/// Durable workflow storage.
pub mod store;
/// Wire types (workflow records, requests).
pub mod types;
