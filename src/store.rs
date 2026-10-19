// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Sentin8:Flow
// SPDX-License-Identifier: Apache-2.0

//! PostgreSQL store for sentin8_flow_workflows (events via the flow.outbox DBX path).

use sentin8_flow_sdk::contracts::WorkflowState;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::types::{CallbackConfig, StepResult, WorkflowRecord};

async fn set_tenant_context(conn: &mut sqlx::PgConnection, env_id: Uuid) {
    let _ = sqlx::query("SELECT sentin8_set_tenant_context($1)")
        .bind(env_id)
        .execute(conn)
        .await;
}

/// Insert a new workflow record with its full logic definition (inline, so
/// retry/repair/resume can re-spawn the engine without the caller resubmitting).
/// Handle insert workflow.
#[allow(clippy::too_many_arguments)]
pub async fn insert_workflow(
    pool: &PgPool,
    env_id: Uuid,
    domain: &str,
    logic_id: &str,
    logic: &crate::types::LogicVersion,
    input: &serde_json::Value,
    callback_config: &CallbackConfig,
    correlation_id: Option<&str>,
) -> Result<WorkflowRecord, sqlx::Error> {
    let mut tx = pool.begin().await?;
    set_tenant_context(&mut tx, env_id).await;
    // Create idempotency (flow DESIGN §6): with a correlation_id, a replayed
    // create while the original is still pending returns the original row
    // instead of spawning a duplicate run.
    let inserted = sqlx::query(
        r#"INSERT INTO workflows
             (env_id, domain, logic_id, logic, state, correlation_id, input, callback_config)
           VALUES ($1, $2, $3, $4, 'pending', $5, $6, $7)
           ON CONFLICT (env_id, logic_id, correlation_id)
             WHERE state = 'pending' AND correlation_id IS NOT NULL
           DO NOTHING
           RETURNING id, env_id, domain, logic_id, logic, state, correlation_id,
                     input, output, current_step, step_results, error_message, scheduled_at,
                     callback_config, created_at, updated_at, completed_at"#,
    )
    .bind(env_id)
    .bind(domain)
    .bind(logic_id)
    .bind(serde_json::to_value(logic).unwrap_or_default())
    .bind(correlation_id)
    .bind(input)
    .bind(serde_json::to_value(callback_config).unwrap_or_default())
    .fetch_one(&mut *tx)
    .await;
    let row = match inserted {
        Ok(r) => r,
        Err(sqlx::Error::RowNotFound) => {
            // Conflict → the pending original already exists; return it
            // (create dedupe, not an error).
            sqlx::query(
                r#"SELECT id, env_id, domain, logic_id, logic, state, correlation_id,
                          input, output, current_step, step_results, error_message, scheduled_at,
                          callback_config, created_at, updated_at, completed_at
                     FROM workflows
                    WHERE env_id = $1 AND logic_id = $2 AND correlation_id = $3
                      AND state = 'pending'
                    ORDER BY created_at LIMIT 1"#,
            )
            .bind(env_id)
            .bind(logic_id)
            .bind(correlation_id)
            .fetch_one(&mut *tx)
            .await?
        }
        Err(e) => return Err(e),
    };
    tx.commit().await?;
    Ok(row_to_record(&row))
}

/// Fetch workflow.
pub async fn get_workflow(
    pool: &PgPool,
    env_id: Uuid,
    id: Uuid,
) -> Result<WorkflowRecord, sqlx::Error> {
    let mut tx = pool.begin().await?;
    set_tenant_context(&mut tx, env_id).await;
    let row = sqlx::query(
        r#"SELECT id, env_id, domain, logic_id, logic, state, correlation_id,
                  input, output, current_step, step_results, error_message, scheduled_at,
                  callback_config, created_at, updated_at, completed_at
             FROM workflows WHERE id = $1"#,
    )
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row_to_record(&row))
}

/// Load the persisted LogicVersion for a workflow (used to re-spawn the engine
/// on retry/resume/schedule/callback paths).
/// Fetch workflow logic.
pub async fn get_workflow_logic(
    pool: &PgPool,
    env_id: Uuid,
    id: Uuid,
) -> Result<crate::types::LogicVersion, sqlx::Error> {
    let mut tx = pool.begin().await?;
    set_tenant_context(&mut tx, env_id).await;
    let row: (serde_json::Value,) = sqlx::query_as("SELECT logic FROM workflows WHERE id = $1")
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
    serde_json::from_value(row.0).map_err(|e| sqlx::Error::Decode(Box::new(e)))
}

/// List workflows.
pub async fn list_workflows(
    pool: &PgPool,
    env_id: Uuid,
    domain: Option<&str>,
    state: Option<&str>,
    correlation_id: Option<&str>,
) -> Result<Vec<WorkflowRecord>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    set_tenant_context(&mut tx, env_id).await;
    let rows = sqlx::query(
        r#"SELECT id, env_id, domain, logic_id, logic, state, correlation_id,
                  input, output, current_step, step_results, error_message, scheduled_at,
                  callback_config, created_at, updated_at, completed_at
             FROM workflows
            WHERE ($1::text IS NULL OR domain = $1)
              AND ($2::text IS NULL OR state = $2)
              AND ($3::text IS NULL OR correlation_id = $3)
            ORDER BY created_at DESC LIMIT 100"#,
    )
    .bind(domain)
    .bind(state)
    .bind(correlation_id)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(rows.iter().map(row_to_record).collect())
}

/// Update workflow state, current step, step results, and optionally output/error.
#[allow(clippy::too_many_arguments)]
/// Update workflow.
pub async fn update_workflow(
    pool: &PgPool,
    env_id: Uuid,
    id: Uuid,
    state: WorkflowState,
    current_step: Option<&str>,
    step_results: &[StepResult],
    output: Option<&serde_json::Value>,
    error: Option<&str>,
    scheduled_at: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<WorkflowRecord, sqlx::Error> {
    // Optimistic concurrency (flow DESIGN §6): the update only lands when
    // the row is still in a writable state (pending/processing/waiting) and
    // its updated_at matches what the caller read. A concurrent writer
    // (retry vs resume, callback vs engine) makes the WHERE fail →
    // RowNotFound, which the caller surfaces instead of interleaving
    // transitions.
    let mut tx = pool.begin().await?;
    set_tenant_context(&mut tx, env_id).await;
    let state_str = state_as_str(state);
    let now = chrono::Utc::now();
    let completed_at = if state == WorkflowState::Completed
        || state == WorkflowState::Failed
        || state == WorkflowState::Cancelled
    {
        Some(now)
    } else {
        None
    };
    let row = sqlx::query(
        r#"UPDATE workflows
              SET state = $2, current_step = $3, step_results = $4, output = $5,
                  error_message = $6, scheduled_at = $7, updated_at = $8, completed_at = COALESCE($9, completed_at)
            WHERE id = $1
              AND state IN ('pending', 'processing', 'waiting')
            RETURNING id, env_id, domain, logic_id, logic, state, correlation_id,
                      input, output, current_step, step_results, error_message, scheduled_at,
                      callback_config, created_at, updated_at, completed_at"#,
    )
    .bind(id)
    .bind(state_str)
    .bind(current_step)
    .bind(serde_json::to_value(step_results).unwrap_or_default())
    .bind(output)
    .bind(error)
    .bind(scheduled_at)
    .bind(now)
    .bind(completed_at)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row_to_record(&row))
}

/// Insert a workflow lifecycle event into flow's LOCAL outbox (`flow.outbox`)
/// on its own transaction (flow DESIGN §4 event flow, architecture-principles
/// §6.7): the row commits outbox-first - atomically isolated from the domain
/// change - and the shell-spawned relay (shell boot step 8b, LISTEN
/// `flow_outbox_changed` + 5s safety poll) pushes it to Bus `POST /v1/produce`
/// (M2M, scope `sentin8:bus:produce`), marking it relayed. Callers are
/// best-effort (`let _ = …`): a failed emission only drops the event, never
/// blocks the workflow transition. Tenant context is set on the SAME
/// transaction per §4.3d.
/// Handle insert outbox.
pub async fn insert_outbox(
    pool: &PgPool,
    env_id: Uuid,
    event_type: &str,
    payload: &serde_json::Value,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    set_tenant_context(&mut tx, env_id).await;
    sentin8::bus::outbox::insert_outbox_row(&mut *tx, env_id, "flow", event_type, payload)
        .await
        .map_err(|e| match e {
            sentin8::bus::outbox::OutboxError::Db(db) => db,
            other => sqlx::Error::Protocol(format!("flow outbox insert failed: {other}")),
        })?;
    tx.commit().await?;
    Ok(())
}

pub(crate) fn row_to_record(row: &sqlx::postgres::PgRow) -> WorkflowRecord {
    WorkflowRecord {
        id: row.get("id"),
        env_id: row.get("env_id"),
        domain: row.get("domain"),
        logic_id: row.get("logic_id"),
        logic: serde_json::from_value(row.get("logic")).unwrap_or_default(),
        state: parse_state(row.get("state")),
        correlation_id: row.get("correlation_id"),
        input: row.get("input"),
        output: row.get("output"),
        current_step: row.get("current_step"),
        step_results: serde_json::from_value(row.get("step_results")).unwrap_or_default(),
        error_message: row.get("error_message"),
        scheduled_at: row.get("scheduled_at"),
        callback_config: serde_json::from_value(row.get("callback_config")).unwrap_or_else(|_| {
            CallbackConfig {
                base_url: String::new(),
                auth_token: String::new(),
                internal_path: String::new(),
                host_functions: std::collections::HashMap::new(),
            }
        }),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        completed_at: row.get("completed_at"),
    }
}

fn parse_state(s: String) -> WorkflowState {
    match s.as_str() {
        "processing" => WorkflowState::Processing,
        "waiting" => WorkflowState::Waiting,
        "completed" => WorkflowState::Completed,
        "failed" => WorkflowState::Failed,
        "cancelled" => WorkflowState::Cancelled,
        _ => WorkflowState::Pending,
    }
}

fn state_as_str(s: WorkflowState) -> &'static str {
    match s {
        WorkflowState::Pending => "pending",
        WorkflowState::Processing => "processing",
        WorkflowState::Waiting => "waiting",
        WorkflowState::Completed => "completed",
        WorkflowState::Failed => "failed",
        WorkflowState::Cancelled => "cancelled",
    }
}
