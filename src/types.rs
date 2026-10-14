// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Sentin8:Flow
// SPDX-License-Identifier: Apache-2.0

//! Flow domain types — server-side types used by the engine, store, and handlers.

use std::collections::HashMap;

use sentin8_flow_sdk::contracts::WorkflowState;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// StepKind identifies a step type in a logic.
/// Type alias for stepkind.
pub type StepKind = String;

/// StepDef defines a single step in a logic.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
/// The stepdef type.
pub struct StepDef {
    /// The name.
    pub name: String,
    /// The kind.
    pub kind: StepKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// The next step.
    pub next_step: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// The config.
    pub config: Option<serde_json::Value>,
    /// `continue` | `fail` | `branch:<step>`
    #[serde(skip_serializing_if = "Option::is_none")]
    /// The on error.
    pub on_error: Option<String>,
}

/// LogicVersion defines a versioned logic with steps.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
/// The logicversion type.
pub struct LogicVersion {
    /// logic identifier.
    pub logic_id: String,
    /// The version.
    pub version: i32,
    /// The steps.
    pub steps: Vec<StepDef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// The correlation id func.
    pub correlation_id_func: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// The expected parameters.
    pub expected_parameters: Option<Vec<String>>,
}

/// StepResult captures the outcome of a single step execution.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
/// The stepresult type.
pub struct StepResult {
    /// The step name.
    pub step_name: String,
    /// The kind.
    pub kind: StepKind,
    /// The status.
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// The output.
    pub output: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// The error.
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// The next step.
    pub next_step: Option<String>,
    /// started time.
    pub started_at: String,
    /// ended time.
    pub ended_at: String,
}

impl StepResult {
    /// Handle success.
    pub fn success(step_name: &str, kind: &str, started: &str, ended: &str) -> Self {
        Self {
            step_name: step_name.into(),
            kind: kind.into(),
            status: "success".into(),
            output: None,
            error: None,
            next_step: None,
            started_at: started.into(),
            ended_at: ended.into(),
        }
    }
    /// Handle failure.
    pub fn failure(step_name: &str, kind: &str, started: &str, ended: &str, err: &str) -> Self {
        Self {
            step_name: step_name.into(),
            kind: kind.into(),
            status: "failed".into(),
            output: None,
            error: Some(err.into()),
            next_step: None,
            started_at: started.into(),
            ended_at: ended.into(),
        }
    }
    /// Handle waiting.
    pub fn waiting(step_name: &str, kind: &str, started: &str, ended: &str) -> Self {
        Self {
            step_name: step_name.into(),
            kind: kind.into(),
            status: "waiting".into(),
            output: None,
            error: None,
            next_step: None,
            started_at: started.into(),
            ended_at: ended.into(),
        }
    }
}

/// CallbackConfig provided by the requesting service.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
/// Configuration for callback.
pub struct CallbackConfig {
    #[serde(default)]
    /// The base URL.
    pub base_url: String,
    #[serde(default)]
    /// The auth token.
    pub auth_token: String,
    #[serde(default)]
    /// The internal path.
    pub internal_path: String,
    #[serde(default)]
    /// The host functions.
    pub host_functions: HashMap<String, String>,
}

/// StepHandlerProfile — how to proxy-call a domain's step handlers.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
/// The stephandlerprofile type.
pub struct StepHandlerProfile {
    /// The domain.
    pub domain: String,
    /// The base URL.
    pub base_url: String,
    /// The auth token.
    pub auth_token: String,
    /// The internal path.
    pub internal_path: String,
}

/// StepContext is the execution context passed to step executors.
#[derive(Debug, Clone)]
/// The stepcontext type.
pub struct StepContext {
    /// workflow identifier.
    pub workflow_id: uuid::Uuid,
    /// env identifier.
    pub env_id: uuid::Uuid,
    /// The domain.
    pub domain: String,
    /// logic identifier.
    pub logic_id: String,
    /// The state.
    pub state: WorkflowState,
    /// The current step.
    pub current_step: String,
    /// The step results.
    pub step_results: Vec<StepResult>,
    /// The input.
    pub input: serde_json::Value,
    /// The output.
    pub output: serde_json::Value,
    /// The parameters.
    pub parameters: HashMap<String, String>,
    /// The user data.
    pub user_data: serde_json::Value,
    /// The step index.
    pub step_index: usize,
    /// The parent workflow's correlation id (inherited by child spawns
    /// from `InitiateInstructionStep` — flow DESIGN §6 create idempotency).
    pub correlation_id: Option<String>,
}

/// CreateWorkflowRequest — body of `POST /v1/workflows`.
/// The `logic`, `input`, and `callback_config` fields accept either a JSON
/// object or a JSON string (from RFW forms which send text fields as strings).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
/// Request body for createworkflow.
pub struct CreateWorkflowRequest {
    /// The domain.
    pub domain: String,
    #[serde(deserialize_with = "deserialize_json_string")]
    /// The logic.
    pub logic: LogicVersion,
    #[serde(default, deserialize_with = "deserialize_json_value_string")]
    /// The input.
    pub input: serde_json::Value,
    #[serde(deserialize_with = "deserialize_callback_config_string")]
    /// The callback config.
    pub callback_config: CallbackConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// correlation identifier.
    pub correlation_id: Option<String>,
}

/// Deserialize a typed struct from a JSON string or object.
/// RFW forms send text fields as strings; this parses them into the target type.
fn deserialize_json_string<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let v = serde_json::Value::deserialize(deserializer)?;
    match v {
        serde_json::Value::String(s) => serde_json::from_str(&s).map_err(serde::de::Error::custom),
        other => serde_json::from_value(other).map_err(serde::de::Error::custom),
    }
}

/// Deserialize a JSON string into a serde_json::Value, or accept the value directly.
fn deserialize_json_value_string<'de, D>(deserializer: D) -> Result<serde_json::Value, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = serde_json::Value::deserialize(deserializer)?;
    match v {
        serde_json::Value::String(s) if s.is_empty() => Ok(serde_json::Value::Null),
        serde_json::Value::String(s) => serde_json::from_str(&s).map_err(serde::de::Error::custom),
        other => Ok(other),
    }
}

/// Deserialize CallbackConfig from a JSON string or object. Empty string → default.
fn deserialize_callback_config_string<'de, D>(deserializer: D) -> Result<CallbackConfig, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = serde_json::Value::deserialize(deserializer)?;
    match v {
        serde_json::Value::String(s) if s.is_empty() => Ok(CallbackConfig::default()),
        serde_json::Value::String(s) => serde_json::from_str(&s).map_err(serde::de::Error::custom),
        other => serde_json::from_value(other).map_err(serde::de::Error::custom),
    }
}

/// RepairWorkflowRequest — body of `POST /v1/workflows/{id}/repair`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
/// Request body for repairworkflow.
pub struct RepairWorkflowRequest {
    /// Index to rewind to (0 = restart from beginning).
    /// The rewind to.
    pub rewind_to: usize,
    /// Optional updated input.
    #[serde(skip_serializing_if = "Option::is_none")]
    /// The input.
    pub input: Option<serde_json::Value>,
}

/// ResumeWorkflowRequest — body of `POST /v1/workflows/{id}/resume`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
/// Request body for resumeworkflow.
pub struct ResumeWorkflowRequest {
    /// Decision/output for the resumed step.
    #[serde(default)]
    /// The decision.
    pub decision: Option<serde_json::Value>,
}

/// WorkflowRecord — the persisted workflow state.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
/// Persisted workflow record.
pub struct WorkflowRecord {
    /// Identifier.
    pub id: uuid::Uuid,
    /// env identifier.
    pub env_id: uuid::Uuid,
    /// The domain.
    pub domain: String,
    /// logic identifier.
    pub logic_id: String,
    #[serde(default)]
    /// The logic.
    pub logic: LogicVersion,
    /// The state.
    pub state: WorkflowState,
    /// correlation identifier.
    pub correlation_id: Option<String>,
    /// The input.
    pub input: serde_json::Value,
    /// The output.
    pub output: Option<serde_json::Value>,
    /// The current step.
    pub current_step: Option<String>,
    /// The step results.
    pub step_results: Vec<StepResult>,
    /// The error message.
    pub error_message: Option<String>,
    /// The scheduled at.
    pub scheduled_at: Option<chrono::DateTime<chrono::Utc>>,
    /// The callback config.
    pub callback_config: CallbackConfig,
    /// created time.
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// updated time.
    pub updated_at: chrono::DateTime<chrono::Utc>,
    /// completed time.
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// RegisterHandlerRequest — body of `POST /v1/handlers/register`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
/// Request body for registerhandler.
pub struct RegisterHandlerRequest {
    /// The domain.
    pub domain: String,
    /// The step kinds.
    pub step_kinds: Vec<String>,
    /// The base URL.
    pub base_url: String,
    /// The auth token.
    pub auth_token: String,
    /// The internal path.
    pub internal_path: String,
}

/// ProxyBackRequest — body sent to the registering service's internal API.
/// `env_id` is the WORKFLOW's tenant — the proxy-back handler MUST set tenant
/// context from it before any domain write (sys design "Proxy-back steps
/// set tenant context"): the M2M bearer identifies the service (machine
/// identity), never the tenant, so the envelope is the only tenant carrier.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
/// Request body for proxyback.
pub struct ProxyBackRequest {
    /// The step kind.
    pub step_kind: String,
    /// workflow identifier.
    pub workflow_id: uuid::Uuid,
    /// env identifier.
    pub env_id: uuid::Uuid,
    /// The step name.
    pub step_name: String,
    /// The input.
    pub input: serde_json::Value,
    /// The user data.
    pub user_data: serde_json::Value,
    /// The step config.
    pub step_config: Option<serde_json::Value>,
}

/// ProxyBackResponse — expected response from the registering service.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
/// Response payload for proxyback.
pub struct ProxyBackResponse {
    /// The status.
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// The output.
    pub output: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// The error.
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// The next step.
    pub next_step: Option<String>,
}
