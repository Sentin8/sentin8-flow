// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Sentin8:Flow
// SPDX-License-Identifier: Apache-2.0

//! 7 built-in generic step executors, handled in-process by Flow.
//!
//! | Kind | Executor |
//! |------|----------|
//! | `BasicStep` | BasicStepExecutor |
//! | `HTTPStep` | HttpStepExecutor |
//! | `HTTPAsyncStep` | HttpAsyncStepExecutor |
//! | `ManualDecisionStep` | ManualDecisionStepExecutor |
//! | `ScheduleInstructionStep` | ScheduleInstructionStepExecutor |
//! | `InitiateInstructionStep` | InitiateInstructionStepExecutor |
//! | `SequenceStep` | SequenceStepExecutor |

use sentin8_flow_sdk::contracts::WorkflowState;
use serde::{Deserialize, Serialize};

use crate::types::{StepContext, StepResult};

/// Outcome of a built-in executor — either proceed or pause the workflow.
pub enum StepOutcome {
    /// Continue to the next step with the result.
    Proceed(StepResult),
    /// Pause the workflow until a decision arrives (optional deadline).
    Waiting(StepResult, Option<chrono::DateTime<chrono::Utc>>),
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

// ── BasicStep ────────────────────────────────────────────────────────────

/// Execute a BasicStep. If the step config contains a `wasmModuleId` and
/// `hook`, calls Func; otherwise it's a no-op (success).
/// Execute basic.
pub async fn execute_basic(
    sc: &StepContext,
    config: Option<&serde_json::Value>,
    func_client: &Option<sentin8_func_sdk::client::FuncClient>,
    callback: &crate::types::CallbackConfig,
) -> StepOutcome {
    let started = now_iso();
    let module_id = config
        .and_then(|c| c.get("wasmModuleId"))
        .and_then(|v| v.as_str());
    let hook = config.and_then(|c| c.get("hook")).and_then(|v| v.as_str());
    if let (Some(client), Some(mid), Some(h)) = (func_client.as_ref(), module_id, hook) {
        let ctx = serde_json::json!({
            "workflow_id" : sc.workflow_id,
            "logic_id" : sc.logic_id,
            "step_name" : sc.current_step,
            "input": sc.input,
            "user_data" : sc.user_data,
        });
        let result = crate::func_integration::execute_wasm(
            client,
            mid,
            h,
            ctx,
            Some(&callback.base_url),
            Some(&callback.auth_token),
            Some(&callback.internal_path),
            Some(&callback.host_functions),
        )
        .await;
        let ended = now_iso();
        return match result {
            Ok(output) => StepOutcome::Proceed(StepResult {
                step_name: sc.current_step.clone(),
                kind: "BasicStep".into(),
                status: "success".into(),
                output: Some(output),
                error: None,
                next_step: None,
                started_at: started,
                ended_at: ended,
            }),
            Err(e) => StepOutcome::Proceed(StepResult::failure(
                &sc.current_step,
                "BasicStep",
                &started,
                &ended,
                &e.to_string(),
            )),
        };
    }
    let ended = now_iso();
    StepOutcome::Proceed(StepResult::success(
        &sc.current_step,
        "BasicStep",
        &started,
        &ended,
    ))
}

// ── HTTPStep ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct HttpStepConfig {
    url: String,
    #[serde(default = "default_method")]
    method: String,
    #[serde(default)]
    headers: std::collections::HashMap<String, String>,
    #[serde(default)]
    body: Option<serde_json::Value>,
    #[serde(default)]
    #[allow(dead_code)]
    timeout_ms: Option<u64>,
}

fn default_method() -> String {
    "GET".into()
}

/// Execute http.
pub async fn execute_http(sc: &StepContext, config: Option<&serde_json::Value>) -> StepOutcome {
    let started = now_iso();
    let cfg: Option<HttpStepConfig> = config.and_then(|c| serde_json::from_value(c.clone()).ok());
    let Some(cfg) = cfg else {
        let ended = now_iso();
        return StepOutcome::Proceed(StepResult::failure(
            &sc.current_step,
            "HTTPStep",
            &started,
            &ended,
            "missing or invalid config",
        ));
    };
    let client = sentin8::middleware::http_client::Sentin8HttpClient::new();
    let mut req = match cfg.method.to_uppercase().as_str() {
        "POST" => client.post(&cfg.url),
        "PUT" => client.put(&cfg.url),
        "DELETE" => client.delete(&cfg.url),
        _ => client.get(&cfg.url),
    };
    for (k, v) in &cfg.headers {
        req = req.header(k, v);
    }
    if let Some(b) = cfg.body.as_ref() {
        req = req.json(b);
    }
    let result = req.send().await;
    let ended = now_iso();
    match result {
        Ok(resp) => {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            let output = serde_json::json!({ "status": status.as_u16(), "body": body });
            StepOutcome::Proceed(StepResult {
                step_name: sc.current_step.clone(),
                kind: "HTTPStep".into(),
                status: if status.is_success() {
                    "success".into()
                } else {
                    "failed".into()
                },
                output: Some(output),
                error: if status.is_success() {
                    None
                } else {
                    Some(format!("HTTP {status}"))
                },
                next_step: None,
                started_at: started,
                ended_at: ended,
            })
        }
        Err(e) => StepOutcome::Proceed(StepResult::failure(
            &sc.current_step,
            "HTTPStep",
            &started,
            &ended,
            &e.to_string(),
        )),
    }
}

// ── HTTPAsyncStep ────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct HttpAsyncStepConfig {
    url: String,
    #[serde(default = "default_method")]
    method: String,
    #[serde(default)]
    headers: std::collections::HashMap<String, String>,
    #[serde(default)]
    body: Option<serde_json::Value>,
}

/// Execute http async.
pub async fn execute_http_async(
    sc: &StepContext,
    config: Option<&serde_json::Value>,
) -> StepOutcome {
    let started = now_iso();
    let cfg: Option<HttpAsyncStepConfig> =
        config.and_then(|c| serde_json::from_value(c.clone()).ok());
    let ended = now_iso();
    let Some(cfg) = cfg else {
        return StepOutcome::Proceed(StepResult::failure(
            &sc.current_step,
            "HTTPAsyncStep",
            &started,
            &ended,
            "missing config",
        ));
    };
    let client = sentin8::middleware::http_client::Sentin8HttpClient::new();
    let mut req = match cfg.method.to_uppercase().as_str() {
        "POST" => client.post(&cfg.url),
        "PUT" => client.put(&cfg.url),
        "DELETE" => client.delete(&cfg.url),
        _ => client.get(&cfg.url),
    };
    for (k, v) in &cfg.headers {
        req = req.header(k, v);
    }
    if let Some(b) = cfg.body.as_ref() {
        req = req.json(b);
    }
    // Fire and forget — workflow waits for callback to /v1/internal/callbacks/async
    let _ = req.send().await;
    StepOutcome::Waiting(
        StepResult::waiting(&sc.current_step, "HTTPAsyncStep", &started, &ended),
        None,
    )
}

// ── ManualDecisionStep ───────────────────────────────────────────────────

/// Execute manual decision.
pub async fn execute_manual_decision(sc: &StepContext) -> StepOutcome {
    let started = now_iso();
    let ended = now_iso();
    StepOutcome::Waiting(
        StepResult::waiting(&sc.current_step, "ManualDecisionStep", &started, &ended),
        None,
    )
}

// ── ScheduleInstructionStep ───────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ScheduleStepConfig {
    #[serde(default)]
    delay_seconds: Option<i64>,
    #[serde(default)]
    scheduled_at: Option<String>,
}

/// Execute schedule instruction.
pub async fn execute_schedule_instruction(
    sc: &StepContext,
    config: Option<&serde_json::Value>,
) -> StepOutcome {
    let started = now_iso();
    let ended = now_iso();
    let cfg: Option<ScheduleStepConfig> =
        config.and_then(|c| serde_json::from_value(c.clone()).ok());
    let scheduled_at = cfg.and_then(|c| {
        if let Some(secs) = c.delay_seconds {
            Some(chrono::Utc::now() + chrono::Duration::seconds(secs))
        } else if let Some(ts) = c.scheduled_at {
            chrono::DateTime::parse_from_rfc3339(&ts)
                .ok()
                .map(|dt| dt.with_timezone(&chrono::Utc))
        } else {
            None
        }
    });
    StepOutcome::Waiting(
        StepResult::waiting(
            &sc.current_step,
            "ScheduleInstructionStep",
            &started,
            &ended,
        ),
        scheduled_at,
    )
}

// ── WaitStep ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct WaitStepConfig {
    /// Delay from now, in seconds.
    #[serde(default)]
    delay_seconds: Option<i64>,
    /// Absolute resume time (RFC 3339) — overrides `delay_seconds`.
    #[serde(default)]
    wait_until: Option<String>,
}

/// Park the workflow until a point in time. The engine parks the workflow
/// in state `waiting` with `scheduled_at` set; Sentin8:Cron resumes due
/// workflows through `POST /v1/internal/schedules/run` (flow DESIGN §5).
/// Distinct from `ScheduleInstructionStep`: Wait is a first-class park for
/// the CURRENT workflow (the wait is the business intent), while schedule
/// is the instruction-shaped alias kept for existing definitions.
pub async fn execute_wait(sc: &StepContext, config: Option<&serde_json::Value>) -> StepOutcome {
    let started = now_iso();
    let ended = now_iso();
    let cfg: Option<WaitStepConfig> = config.and_then(|c| serde_json::from_value(c.clone()).ok());
    let scheduled_at = cfg.and_then(|c| {
        if let Some(secs) = c.delay_seconds {
            Some(chrono::Utc::now() + chrono::Duration::seconds(secs))
        } else if let Some(ts) = c.wait_until {
            chrono::DateTime::parse_from_rfc3339(&ts)
                .ok()
                .map(|dt| dt.with_timezone(&chrono::Utc))
        } else {
            None
        }
    });
    if scheduled_at.is_none() {
        return StepOutcome::Proceed(StepResult::failure(
            &sc.current_step,
            "WaitStep",
            &started,
            &ended,
            "missing config — one of delay_seconds or wait_until is required",
        ));
    }
    StepOutcome::Waiting(
        StepResult::waiting(&sc.current_step, "WaitStep", &started, &ended),
        scheduled_at,
    )
}

// ── InitiateInstructionStep ──────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
struct InitiateStepConfig {
    /// The child domain (the owning service's identity, e.g.
    /// `sentin8:sys`) — the child workflow's proxy-back target. Defaults
    /// to the parent's domain when omitted.
    #[serde(default)]
    domain: Option<String>,
    logic_id: String,
    #[serde(default)]
    input: serde_json::Value,
}

/// Creates a child workflow record via the store. Returns the child ID as output.
/// Execute initiate instruction.
pub async fn execute_initiate_instruction(
    sc: &StepContext,
    config: Option<&serde_json::Value>,
    pool: &sqlx::PgPool,
    callback: &crate::types::CallbackConfig,
) -> StepOutcome {
    let started = now_iso();
    let ended = now_iso();
    let cfg: Option<InitiateStepConfig> =
        config.and_then(|c| serde_json::from_value(c.clone()).ok());
    let Some(cfg) = cfg else {
        return StepOutcome::Proceed(StepResult::failure(
            &sc.current_step,
            "InitiateInstructionStep",
            &started,
            &ended,
            "missing config",
        ));
    };
    // The child inherits the parent's correlation_id (the spawn is part of
    // the same business transaction — a replayed parent create returns the
    // original pending child instead of spawning a duplicate run; flow
    // DESIGN §6 create idempotency).
    let child = crate::store::insert_workflow(
        pool,
        sc.env_id,
        cfg.domain.as_deref().unwrap_or(&sc.domain),
        &cfg.logic_id,
        &crate::types::LogicVersion::default(),
        &cfg.input,
        callback,
        sc.correlation_id.as_deref(),
    )
    .await;
    match child {
        Ok(record) => StepOutcome::Proceed(StepResult {
            step_name: sc.current_step.clone(),
            kind: "InitiateInstructionStep".into(),
            status: "success".into(),
            output: Some(serde_json::json!({ "workflow_id" : record.id })),
            error: None,
            next_step: None,
            started_at: started,
            ended_at: ended,
        }),
        Err(e) => StepOutcome::Proceed(StepResult::failure(
            &sc.current_step,
            "InitiateInstructionStep",
            &started,
            &ended,
            &e.to_string(),
        )),
    }
}

// ── SequenceStep ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct SequenceStepConfig {
    /// Sequence key/namespace.
    #[serde(default)]
    key: String,
}

/// Execute sequence.
pub async fn execute_sequence(
    sc: &StepContext,
    config: Option<&serde_json::Value>,
    pool: &sqlx::PgPool,
) -> StepOutcome {
    let started = now_iso();
    let ended = now_iso();
    let cfg: Option<SequenceStepConfig> =
        config.and_then(|c| serde_json::from_value(c.clone()).ok());
    let key = cfg.map(|c| c.key).unwrap_or_else(|| "default".into());
    // Use a Postgres sequence for monotonic numbers per (org, env, key).
    let seq_name = format!("sentin8_flow_seq_{}", key.replace('-', "_"));
    let mut conn = match pool.acquire().await {
        Ok(c) => c,
        Err(e) => {
            return StepOutcome::Proceed(StepResult::failure(
                &sc.current_step,
                "SequenceStep",
                &started,
                &ended,
                &e.to_string(),
            ))
        }
    };
    let _ = sqlx::query("SELECT sentin8_set_tenant_context($1)")
        .bind(sc.env_id)
        .execute(&mut *conn)
        .await;
    let _ = sqlx::query(&format!("CREATE SEQUENCE IF NOT EXISTS {seq_name}"))
        .execute(&mut *conn)
        .await;
    let row = sqlx::query(&format!("SELECT nextval('{seq_name}') AS val"))
        .fetch_one(&mut *conn)
        .await;
    match row {
        Ok(r) => {
            use sqlx::Row;
            let val: i64 = r.get("val");
            StepOutcome::Proceed(StepResult {
                step_name: sc.current_step.clone(),
                kind: "SequenceStep".into(),
                status: "success".into(),
                output: Some(serde_json::json!({ "sequence": val })),
                error: None,
                next_step: None,
                started_at: started,
                ended_at: ended,
            })
        }
        Err(e) => StepOutcome::Proceed(StepResult::failure(
            &sc.current_step,
            "SequenceStep",
            &started,
            &ended,
            &e.to_string(),
        )),
    }
}

// ── Dispatch ─────────────────────────────────────────────────────────────

/// Returns true if the step kind is one of the 7 generic built-in types.
/// Handle is generic.
pub fn is_generic(kind: &str) -> bool {
    matches!(
        kind,
        "BasicStep"
            | "HTTPStep"
            | "HTTPAsyncStep"
            | "ManualDecisionStep"
            | "ScheduleInstructionStep"
            | "WaitStep"
            | "InitiateInstructionStep"
            | "SequenceStep"
    )
}

/// Execute a generic step. Returns a StepOutcome.
#[allow(clippy::too_many_arguments)]
/// Execute generic.
pub async fn execute_generic(
    sc: &StepContext,
    kind: &str,
    config: Option<&serde_json::Value>,
    pool: &sqlx::PgPool,
    func_client: &Option<sentin8_func_sdk::client::FuncClient>,
    callback: &crate::types::CallbackConfig,
) -> StepOutcome {
    match kind {
        "BasicStep" => execute_basic(sc, config, func_client, callback).await,
        "HTTPStep" => execute_http(sc, config).await,
        "HTTPAsyncStep" => execute_http_async(sc, config).await,
        "ManualDecisionStep" => execute_manual_decision(sc).await,
        "ScheduleInstructionStep" => execute_schedule_instruction(sc, config).await,
        "WaitStep" => execute_wait(sc, config).await,
        "InitiateInstructionStep" => execute_initiate_instruction(sc, config, pool, callback).await,
        "SequenceStep" => execute_sequence(sc, config, pool).await,
        _ => StepOutcome::Proceed(StepResult::failure(
            &sc.current_step,
            kind,
            &now_iso(),
            &now_iso(),
            "unknown generic step",
        )),
    }
}

/// Marker for waiting state used by the engine.
/// Handle state waiting.
pub fn state_waiting() -> WorkflowState {
    WorkflowState::Waiting
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generic_step_classifier_covers_builtins() {
        assert!(is_generic("HTTPStep"));
        assert!(is_generic("SequenceStep"));
        assert!(is_generic("WaitStep"));
        assert!(!is_generic("PeriodCalculationStep"));
        assert!(!is_generic("UnknownStep"));
    }
}
