// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Sentin8:Flow
// SPDX-License-Identifier: Apache-2.0

//! Flow HTTP handlers - workflow CRUD, retry, repair, resume, handler registration.

use sentin8::middleware::health::LiveHealth;
use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use sentin8_flow_sdk::contracts::WorkflowState;
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::admin_ui::TargetEnvQuery;
use crate::auth;
use crate::engine::Engine;
use crate::proxy_back::HandlerRegistry;
use crate::store;
use crate::types::{
    CreateWorkflowRequest, RegisterHandlerRequest, RepairWorkflowRequest, ResumeWorkflowRequest,
    WorkflowRecord,
};

#[derive(Clone)]
/// app state.
pub struct AppState {
    /// The config.
    pub config: Arc<crate::config::FlowConfig>,
    /// The pool.
    pub pool: sqlx::PgPool,
    /// The engine.
    pub engine: Arc<Engine>,
    /// The registry.
    pub registry: HandlerRegistry,
    /// Shield engine ( - ): additional gate after scope.
    /// The policy.
    pub policy: Arc<crate::policy::Engine>,
    /// auth DESIGN §4.3f: the boot-loaded workload registry map - the
    /// authorization record for certificate-only platform-workload
    /// calls (sys's delegation presents the `sys-flow` certificate;
    /// scopes ride the map, not a token).
    pub workload_registry: sentin8::auth::workload_registry::WorkloadMap,
    /// Shared Sentin8:Policy admin surface ( - one
    /// implementation for every service; mounted in server::routes).
    /// The admin.
    pub admin: sentin8::shield::admin::SharedAdminState,
}

// ── Create workflow ──────────────────────────────────────────────────────

/// `POST /v1/workflows` - create + start workflow.
#[utoipa::path(
    post,
    operation_id = "workflow.create",
    path = "/v1/workflows",
    request_body = CreateWorkflowRequest,
    responses(
        (status = 200, description = "Workflow created", body = WorkflowRecord),
        (status = 403, description = "Missing flow:write scope"),
    ),
    security(("sentin8:flow:write" = [])),
    tag = "flow",
)]
/// Create workflow.
pub async fn create_workflow(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TargetEnvQuery>,
    Json(req): Json<CreateWorkflowRequest>,
) -> Result<Json<WorkflowRecord>, (StatusCode, String)> {
    let a = auth::verify_cert_or_token(&state.workload_registry, &state.config, &headers).await?;
    crate::policy::require_policy(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("sentin8:flow:write"),
    )
    .await?;
    let env_id = q
        .target_env_id
        .as_deref()
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .unwrap_or_else(|| a.env_id());
    let record = store::insert_workflow(
        &state.pool,
        env_id,
        &req.domain,
        &req.logic.logic_id,
        &req.logic,
        &req.input,
        &req.callback_config,
        req.correlation_id.as_deref(),
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    // Emit workflow.created outbox event.
    let _ = store::insert_outbox(
        &state.pool, env_id, "sentin8:flow:workflow:created",
        &serde_json::json!({ "workflow_id": record.id, "logic_id": record.logic_id, "domain": record.domain }),
    ).await;
    // Start execution.
    let engine = state.engine.clone();
    let logic = req.logic;
    let _pool = state.pool.clone();
    let record_clone = record.clone();
    tokio::spawn(async move {
        engine.execute(env_id, record_clone, &logic).await;
    });
    Ok(Json(record))
}

// ── List workflows ───────────────────────────────────────────────────────

#[derive(Debug, Deserialize, ToSchema)]
/// The listquery type.
pub struct ListQuery {
    #[serde(default)]
    /// The domain.
    pub domain: Option<String>,
    #[serde(default)]
    /// The state.
    pub state: Option<String>,
    #[serde(default)]
    /// correlation identifier.
    pub correlation_id: Option<String>,
    #[serde(default)]
    /// Target environment - same contract as TargetEnvQuery (query param
    /// wins over the token's env; user tokens may carry no env at all,
    /// which then resolves to the system env - the platform default).
    pub target_env_id: Option<String>,
}

/// Shared env resolution for workflow handlers: an explicit `target_env_id`
/// query parameter wins; otherwise the token's env claim; a user token
/// without an env claim resolves to the system env (platform default -
/// `create_workflow`'s existing behavior, now uniform across the surface).
fn resolve_target_env(target_env_id: Option<&str>, a: &sentin8::auth::client::AuthContext) -> Uuid {
    target_env_id
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .unwrap_or_else(|| a.env_id())
}

/// `GET /v1/workflows`
#[utoipa::path(
    get,
    operation_id = "workflow.list",
    path = "/v1/workflows",
    params(("domain" = Option<String>, Query), ("state" = Option<String>, Query), ("correlation_id" = Option<String>, Query)),
    responses(
        (status = 200, description = "List of workflows", body = [WorkflowRecord]),
        (status = 403, description = "Missing flow:read scope"),
    ),
    security(("sentin8:flow:read" = [])),
    tag = "flow",
)]
/// List workflows.
pub async fn list_workflows(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<ListQuery>,
) -> Result<Json<Vec<WorkflowRecord>>, (StatusCode, String)> {
    let a = auth::verify_cert_or_token(&state.workload_registry, &state.config, &headers).await?;
    crate::policy::require_policy(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("sentin8:flow:read"),
    )
    .await?;
    let records = store::list_workflows(
        &state.pool,
        resolve_target_env(q.target_env_id.as_deref(), &a),
        q.domain.as_deref(),
        q.state.as_deref(),
        q.correlation_id.as_deref(),
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(records))
}

// ── Get workflow ─────────────────────────────────────────────────────────

/// `GET /v1/workflows/{id}`
#[utoipa::path(
    get,
    operation_id = "workflow.get",
    path = "/v1/workflows/{id}",
    params(("id" = Uuid, Path, description = "Workflow ID")),
    responses(
        (status = 200, description = "Workflow", body = WorkflowRecord),
        (status = 404, description = "Workflow not found"),
        (status = 403, description = "Missing flow:read scope"),
    ),
    security(("sentin8:flow:read" = [])),
    tag = "flow",
)]
/// Fetch workflow.
pub async fn get_workflow(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TargetEnvQuery>,
    Path(id): Path<Uuid>,
) -> Result<Json<WorkflowRecord>, (StatusCode, String)> {
    let a = auth::verify_cert_or_token(&state.workload_registry, &state.config, &headers).await?;
    crate::policy::require_policy(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("sentin8:flow:read"),
    )
    .await?;
    let env_id = resolve_target_env(q.target_env_id.as_deref(), &a);
    let record = store::get_workflow(&state.pool, env_id, id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    Ok(Json(record))
}

// ── Retry ─────────────────────────────────────────────────────────────────

/// `POST /v1/workflows/{id}/retry` - retry a failed workflow from the failed step.
#[utoipa::path(
    post,
    operation_id = "workflow.retry",
    path = "/v1/workflows/{id}/retry",
    params(("id" = Uuid, Path, description = "Workflow ID")),
    responses(
        (status = 200, description = "Workflow retried", body = WorkflowRecord),
        (status = 404, description = "Workflow not found"),
        (status = 403, description = "Missing flow:write scope"),
    ),
    security(("sentin8:flow:write" = [])),
    tag = "flow",
)]
/// Retry workflow.
pub async fn retry_workflow(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TargetEnvQuery>,
    Path(id): Path<Uuid>,
) -> Result<Json<WorkflowRecord>, (StatusCode, String)> {
    let a = auth::verify_cert_or_token(&state.workload_registry, &state.config, &headers).await?;
    crate::policy::require_policy(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("sentin8:flow:write"),
    )
    .await?;
    let env_id = resolve_target_env(q.target_env_id.as_deref(), &a);
    let record = store::get_workflow(&state.pool, env_id, id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    if record.state != WorkflowState::Failed {
        return Err((
            StatusCode::BAD_REQUEST,
            "only failed workflows can be retried".into(),
        ));
    }
    // Reset to pending, keep current_step so engine resumes from there.
    let record = store::update_workflow(
        &state.pool,
        env_id,
        id,
        WorkflowState::Pending,
        record.current_step.as_deref(),
        &record.step_results,
        None,
        None,
        None,
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    // Emit workflow.retried outbox event.
    let _ = store::insert_outbox(
        &state.pool, env_id, "sentin8:flow:workflow:retried",
        &serde_json::json!({ "workflow_id": record.id, "logic_id": record.logic_id, "current_step": record.current_step }),
    ).await;
    // Re-spawn the engine from the persisted logic - the workflow resumes
    // from current_step.
    let engine = state.engine.clone();
    let env = env_id;
    let pool = state.pool.clone();
    let wid = record.id;
    let exec_record = record.clone();
    tokio::spawn(async move {
        match store::get_workflow_logic(&pool, env, wid).await {
            Ok(logic) => {
                engine.execute(env, exec_record, &logic).await;
            }
            Err(e) => tracing::error!(error = %e, "failed to load logic for retry"),
        }
    });
    Ok(Json(record))
}

// ── Repair ────────────────────────────────────────────────────────────────

/// `POST /v1/workflows/{id}/repair` - rewind to a step and reprocess.
#[utoipa::path(
    post,
    operation_id = "workflow.repair",
    path = "/v1/workflows/{id}/repair",
    params(("id" = Uuid, Path, description = "Workflow ID")),
    request_body = RepairWorkflowRequest,
    responses(
        (status = 200, description = "Workflow repaired", body = WorkflowRecord),
        (status = 404, description = "Workflow not found"),
        (status = 403, description = "Missing flow:write scope"),
    ),
    security(("sentin8:flow:write" = [])),
    tag = "flow",
)]
/// Repair workflow.
pub async fn repair_workflow(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TargetEnvQuery>,
    Path(id): Path<Uuid>,
    Json(req): Json<RepairWorkflowRequest>,
) -> Result<Json<WorkflowRecord>, (StatusCode, String)> {
    let a = auth::verify_cert_or_token(&state.workload_registry, &state.config, &headers).await?;
    crate::policy::require_policy(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("sentin8:flow:write"),
    )
    .await?;
    let env_id = resolve_target_env(q.target_env_id.as_deref(), &a);
    let record = store::get_workflow(&state.pool, env_id, id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    // Truncate step results to rewind point.
    let truncated: Vec<_> = record
        .step_results
        .into_iter()
        .take(req.rewind_to)
        .collect();
    let input = req.input.unwrap_or(record.input);
    let record = store::update_workflow(
        &state.pool,
        env_id,
        id,
        WorkflowState::Pending,
        truncated.last().map(|r| r.step_name.as_str()),
        &truncated,
        Some(&input),
        None,
        None,
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    // Emit workflow.repaired outbox event.
    let _ = store::insert_outbox(
        &state.pool, env_id, "sentin8:flow:workflow:repaired",
        &serde_json::json!({ "workflow_id": record.id, "logic_id": record.logic_id, "rewind_to": req.rewind_to }),
    ).await;
    Ok(Json(record))
}

// ── Cancel ───────────────────────────────────────────────────────────────

/// `POST /v1/workflows/{id}/cancel`
#[utoipa::path(
    post,
    operation_id = "workflow.cancel",
    path = "/v1/workflows/{id}/cancel",
    params(("id" = Uuid, Path, description = "Workflow ID")),
    responses(
        (status = 200, description = "Workflow cancelled", body = WorkflowRecord),
        (status = 404, description = "Workflow not found"),
        (status = 403, description = "Missing flow:write scope"),
    ),
    security(("sentin8:flow:write" = [])),
    tag = "flow",
)]
/// Cancel workflow.
pub async fn cancel_workflow(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TargetEnvQuery>,
    Path(id): Path<Uuid>,
) -> Result<Json<WorkflowRecord>, (StatusCode, String)> {
    let a = auth::verify_cert_or_token(&state.workload_registry, &state.config, &headers).await?;
    crate::policy::require_policy(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("sentin8:flow:write"),
    )
    .await?;
    let env_id = resolve_target_env(q.target_env_id.as_deref(), &a);
    let record = store::get_workflow(&state.pool, env_id, id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    // Terminal-state guard (flow DESIGN transitions): cancel on a
    // completed/failed/already-cancelled workflow is a 409 - the engine
    // may complete a run between the caller's create/read and this
    // cancel (never surface the write-skim as a 500).
    if matches!(
        record.state,
        WorkflowState::Completed | WorkflowState::Failed | WorkflowState::Cancelled
    ) {
        return Err((
            StatusCode::CONFLICT,
            format!(
                "workflow {} is already terminal ({:?})",
                record.id, record.state
            ),
        ));
    }
    let record = store::update_workflow(
        &state.pool,
        env_id,
        id,
        WorkflowState::Cancelled,
        record.current_step.as_deref(),
        &record.step_results,
        record.output.as_ref(),
        Some("cancelled by request"),
        None,
    )
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => (
            StatusCode::CONFLICT,
            "workflow changed state concurrently".to_string(),
        ),
        other => (StatusCode::INTERNAL_SERVER_ERROR, other.to_string()),
    })?;
    let _ = store::insert_outbox(
        &state.pool,
        env_id,
        "sentin8:flow:workflow:cancelled",
        &serde_json::json!({ "workflow_id": record.id, "logic_id": record.logic_id }),
    )
    .await;
    Ok(Json(record))
}

// ── Resume ────────────────────────────────────────────────────────────────

/// `POST /v1/workflows/{id}/resume` - resume from waiting state (manual decision, async HTTP, schedule).
#[utoipa::path(
    post,
    operation_id = "workflow.resume",
    path = "/v1/workflows/{id}/resume",
    params(("id" = Uuid, Path, description = "Workflow ID")),
    request_body = ResumeWorkflowRequest,
    responses(
        (status = 200, description = "Workflow resumed", body = WorkflowRecord),
        (status = 404, description = "Workflow not found"),
        (status = 403, description = "Missing flow:write scope"),
    ),
    security(("sentin8:flow:write" = [])),
    tag = "flow",
)]
/// Resume workflow.
pub async fn resume_workflow(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TargetEnvQuery>,
    Path(id): Path<Uuid>,
    Json(req): Json<ResumeWorkflowRequest>,
) -> Result<Json<WorkflowRecord>, (StatusCode, String)> {
    let a = auth::verify_cert_or_token(&state.workload_registry, &state.config, &headers).await?;
    crate::policy::require_policy(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("sentin8:flow:write"),
    )
    .await?;
    let env_id = resolve_target_env(q.target_env_id.as_deref(), &a);
    let record = store::get_workflow(&state.pool, env_id, id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    if record.state != WorkflowState::Waiting {
        return Err((
            StatusCode::BAD_REQUEST,
            "only waiting workflows can be resumed".into(),
        ));
    }
    // If decision provided, append it to step results output.
    let mut step_results = record.step_results.clone();
    if let Some(last) = step_results.last_mut() {
        if last.status == "waiting" {
            last.status = "success".into();
            last.output = req.decision.clone();
        }
    }
    let record = store::update_workflow(
        &state.pool,
        env_id,
        id,
        WorkflowState::Pending,
        record.current_step.as_deref(),
        &step_results,
        record.output.as_ref(),
        None,
        None,
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    // Re-spawn the engine from the persisted logic - the workflow resumes
    // from current_step with the decision applied to the last step result.
    let engine = state.engine.clone();
    let env = env_id;
    let pool = state.pool.clone();
    let wid = record.id;
    let exec_record = record.clone();
    tokio::spawn(async move {
        match store::get_workflow_logic(&pool, env, wid).await {
            Ok(logic) => {
                engine.execute(env, exec_record, &logic).await;
            }
            Err(e) => tracing::error!(error = %e, "failed to load logic for resume"),
        }
    });
    Ok(Json(record))
}

// ── Handler registration ─────────────────────────────────────────────────

/// `POST /v1/handlers/register` - register a domain's step handler profile.
#[utoipa::path(
    post,
    path = "/v1/handlers/register",
    request_body = RegisterHandlerRequest,
    responses(
        (status = 204, description = "Handler registered"),
        (status = 403, description = "Missing flow:write scope"),
    ),
    security(("sentin8:flow:write" = [])),
    tag = "flow",
)]
/// Register handler.
pub async fn register_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<RegisterHandlerRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    let _a = auth::verify_cert_or_token(&state.workload_registry, &state.config, &headers).await?;
    crate::policy::require_policy(
        &state.policy,
        &_a,
        &headers,
        sentin8::shield::EvalTarget::new("sentin8:flow:write"),
    )
    .await?;
    let profile = crate::types::StepHandlerProfile {
        domain: req.domain,
        base_url: req.base_url,
        auth_token: req.auth_token,
        internal_path: req.internal_path,
    };
    state.registry.register(profile).await;
    // step_kinds list is informational - the registry maps by domain, not per-kind.
    Ok(StatusCode::NO_CONTENT)
}

// ── Internal: schedule run (Cron proxy-back) ────────────────────────────

/// `POST /v1/internal/schedules/run` - Cron calls this to resume due scheduled workflows.
/// Handle internal schedule run.

// ── Typed proxy-back replies (OAPEC-9) ──────────────────────────────────

/// The `POST /v1/internal/schedules/run` reply.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct ScheduleRunResponse {
    /// The due workflows resumed in this pass.
    pub resumed: usize,
}

/// The `POST /v1/internal/callbacks/async` reply.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct AsyncCallbackResponse {
    /// `true` when the waiting step advanced.
    pub resumed: bool,
}

#[utoipa::path(
    post,
    path = "/v1/internal/schedules/run",
    responses(
        (status = 200, description = "Due scheduled workflows resumed (count returned)", body = ScheduleRunResponse),
        (status = 401, description = "Caller authentication failed (cert identity or bearer)"),
    ),
    security(("sentin8:flow:write" = [])),
    tag = "internal",
)]
/// Resume the due scheduled workflows (cron proxy-back).
pub async fn internal_schedule_run(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ScheduleRunResponse>, (StatusCode, String)> {
    let _a = auth::verify_cert_or_token(&state.workload_registry, &state.config, &headers).await?;
    // Find waiting workflows with scheduled_at <= now.
    let rows: Vec<(Uuid, Uuid)> = sqlx::query_as(
        r#"SELECT id, env_id
             FROM workflows
            WHERE state = 'waiting' AND scheduled_at IS NOT NULL AND scheduled_at <= now()
            LIMIT 50"#,
    )
    .fetch_all(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let count = rows.len();
    for (id, env_id) in rows {
        // Resume: set back to pending. The engine will re-execute from current_step.
        let record = match sqlx::query(
                "UPDATE workflows SET state = 'pending', scheduled_at = NULL, updated_at = now() WHERE id = $1 RETURNING *",
            )
            .bind(id)
            .fetch_one(&state.pool)
            .await
        {
            Ok(r) => crate::store::row_to_record(&r),
            Err(e) => {
                tracing::error!(error = %e, "failed to reset scheduled workflow");
                continue;
            }
        };
        let _ = store::insert_outbox(
            &state.pool,
            env_id,
            "sentin8:flow:workflow:schedule:resumed",
            &serde_json::json!({"workflow_id" : id}),
        )
        .await;
        // Re-spawn the engine from the persisted logic.
        let engine = state.engine.clone();
        let pool = state.pool.clone();
        tokio::spawn(async move {
            match store::get_workflow_logic(&pool, env_id, id).await {
                Ok(logic) => {
                    engine.execute(env_id, record, &logic).await;
                }
                Err(e) => tracing::error!(error = %e, "failed to load logic for schedule resume"),
            }
        });
    }
    Ok(Json(ScheduleRunResponse { resumed: count }))
}

// ── Internal: async callback ────────────────────────────────────────────

#[derive(Debug, Deserialize, utoipa::ToSchema)]
/// Request body for asynccallback.
pub struct AsyncCallbackRequest {
    /// workflow identifier.
    pub workflow_id: Uuid,
    /// The status.
    pub status: String,
    #[serde(default)]
    /// The output.
    pub output: Option<serde_json::Value>,
    #[serde(default)]
    /// The error.
    pub error: Option<String>,
}

/// `POST /v1/internal/callbacks/async` - external callback to resume a waiting workflow.
/// Handle internal async callback.
#[utoipa::path(
    post,
    path = "/v1/internal/callbacks/async",
    request_body = AsyncCallbackRequest,
    responses(
        (status = 200, description = "Workflow resumed (waiting step advanced with the callback status/output)", body = AsyncCallbackResponse),
        (status = 404, description = "Unknown workflow"),
        (status = 409, description = "Workflow is not in a waiting state"),
        (status = 401, description = "Caller authentication failed (cert identity or bearer)"),
    ),
    security(("sentin8:flow:write" = [])),
    tag = "internal",
)]
pub async fn internal_async_callback(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<AsyncCallbackRequest>,
) -> Result<Json<AsyncCallbackResponse>, (StatusCode, String)> {
    let a = auth::verify_cert_or_token(&state.workload_registry, &state.config, &headers).await?;
    let env_id = a.env_id();
    let record = store::get_workflow(&state.pool, env_id, req.workflow_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    if record.state != WorkflowState::Waiting {
        return Err((StatusCode::BAD_REQUEST, "workflow is not waiting".into()));
    }
    let mut step_results = record.step_results.clone();
    if let Some(last) = step_results.last_mut() {
        last.status = req.status.clone();
        last.output = req.output.clone();
        last.error = req.error.clone();
    }
    let record = store::update_workflow(
        &state.pool,
        env_id,
        req.workflow_id,
        WorkflowState::Pending,
        record.current_step.as_deref(),
        &step_results,
        record.output.as_ref(),
        req.error.as_deref(),
        None,
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    // Re-spawn the engine from the persisted logic.
    let engine = state.engine.clone();
    let pool = state.pool.clone();
    tokio::spawn(async move {
        match store::get_workflow_logic(&pool, env_id, req.workflow_id).await {
            Ok(logic) => {
                engine.execute(env_id, record, &logic).await;
            }
            Err(e) => tracing::error!(error = %e, "failed to load logic for async callback resume"),
        }
    });
    Ok(Json(AsyncCallbackResponse { resumed: true }))
}

// ── Health ───────────────────────────────────────────────────────────────

/// Handle health.
#[utoipa::path(
    get,
    path = "/v1/health",
    responses(
        (status = 200, description = "Liveness (version, build, active connections)", body = sentin8::middleware::health::LiveHealth),
    ),
    security(),
    tag = "meta",
)]
pub async fn health() -> Json<LiveHealth> {
    Json(LiveHealth {
        status: "ok".to_string(),
        service: Some("flow".to_string()),
        version: Some(sentin8::build_info::id()),
        build: Some(sentin8::build_info::build_hash().to_string()),
        active_connections: sentin8::middleware::traffic_counter::process_active_connections()
            .map(|n| n as i64),
    })
}
