// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Sentin8:Flow
// SPDX-License-Identifier: Apache-2.0

//! Sentin8:Flow Admin UI handlers - RFW definitions.
//! Served from `GET /v1/admin/ui/{page}`, rendered by Sentin8:Dash at runtime.

use crate::handlers::AppState;
use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    Json,
};
use sentin8::rfw::page::*;
use sentin8::rfw::response::AdminUiResponse;
use sentin8::rfw::widget::*;
use serde::Deserialize;
use sqlx::Row;

#[derive(Deserialize, utoipa::IntoParams, utoipa::ToSchema)]
/// The targetenvquery type.
pub struct TargetEnvQuery {
    #[serde(default)]
    /// target env identifier.
    pub target_env_id: Option<String>,
}

/// Fetch admin ui.
#[utoipa::path(
    get,
    path = "/v1/admin/ui/{page}",
    params(
        ("page" = String, Path, description = "RFW page name"),
        ("target_env_id" = String, Query, description = "Target env (RLS context)")
    ),
    responses(
        (status = 200, description = "RFW page definition (rendered by Sentin8:Dash)", body = serde_json::Value),
        (status = 403, description = "Caller lacks a flow read/admin scope"),
        (status = 404, description = "Unknown page"),
    ),
    security(("sentin8:flow:read" = [])),
    tag = "admin-ui",
)]
pub async fn get_admin_ui(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(_q): Query<TargetEnvQuery>,
    Path(page): Path<String>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    let _a = crate::auth::require_any_scope(
        &state.config,
        &headers,
        &[
            "sentin8:flow:read",
            "sentin8:flow:write",
            "sentin8:sys:admin",
            "sentin8:sys:org:admin",
            "sentin8:sys:env:admin",
        ],
    )
    .await?;
    let def = match page.as_str() {
        "overview" => AdminPage::new("Flow Overview").widget(card("Summary","Active workflows, running executions, failed steps.")).build(),
        "workflows" => AdminPage::new("Workflows")
            .action(link("Create Workflow", "workflows/new"))
            .widget(table_with_actions(vec![col("id","Workflow ID"), cols("name","Name"), cols("status","Status"), cols("current_step","Current Step")], "/workflows", vec![
                row_action_post("Retry", "workflows/{id}/retry"),
                row_action_post("Cancel", "workflows/{id}/cancel"),
            ]))
            .build(),
        "workflows/new" => AdminPage::new("Create Workflow")
            .widget(RemoteWidget::Form(WidgetForm {
                fields: vec![
                    WidgetFormField {
                        key: "domain".into(), label: "Domain".into(),
                        field_type: FormFieldType::Text, required: true,
                        placeholder: Some("e.g. ledger".into()),
                        default_value: None, options: None, validation: None, width: None,
                        help_text: Some("The domain this workflow belongs to.".into()), visible_if: None,
                    },
                    WidgetFormField {
                        key: "logic".into(), label: "Logic Definition (JSON)".into(),
                        field_type: FormFieldType::Text, required: true,
                        placeholder: Some("{\"logic_id\":\"...\",\"version\":1,\"steps\":[...]}".into()),
                        default_value: None, options: None, validation: None, width: None,
                        help_text: Some("Logic definition JSON (logic_id, version, steps array).".into()), visible_if: None,
                    },
                    WidgetFormField {
                        key: "input".into(), label: "Input (JSON)".into(),
                        field_type: FormFieldType::Text, required: false,
                        placeholder: Some("{}".into()),
                        default_value: None, options: None, validation: None, width: None,
                        help_text: Some("Input payload as JSON.".into()), visible_if: None,
                    },
                    WidgetFormField {
                        key: "callback_config".into(), label: "Callback Config (JSON)".into(),
                        field_type: FormFieldType::Text, required: false,
                        placeholder: Some("{\"base_url\":\"...\",\"auth_token\":\"...\",\"internal_path\":\"...\"}".into()),
                        default_value: None, options: None, validation: None, width: None,
                        help_text: Some("Callback configuration for workflow step callbacks.".into()), visible_if: None,
                    },
                ],
                submit_endpoint: "/v1/workflows".into(),
                submit_label: "Create Workflow".into(),
                cancel_label: "Cancel".into(),
                method: FormMethod::Post,
            }))
            .build(),
        "steps" => AdminPage::new("Step Executions")
            .widget(table_with_actions(vec![col("id","Step ID"), cols("workflow_id","Workflow"), cols("step_name","Step"), cols("status","Status"), cols("started_at","Started"), cols("completed_at","Completed")], "/steps", vec![
                row_action_post("Retry", "workflows/{workflow_id}/steps/{id}/retry"),
            ]))
            .build(),
 // ── Policy Studio - shared page
 // constructors; the guard guarantees `Some` ( dispatch).
        p if sentin8::rfw::policy_page("sentin8:flow", p).is_some() => sentin8::rfw
            ::policy_page("sentin8:flow", p)
            .expect("policy_page checked in guard")
            .build(),
        _ => return Err((axum::http::StatusCode::NOT_FOUND, format!("Unknown admin UI page: {page}"))),
    };
    Ok(Json(
        serde_json::to_value(AdminUiResponse::page(def)).unwrap(),
    ))
}

/// `GET /v1/admin/{resource}` - returns a JSON array of admin data rows.
/// Fetch admin data.
#[utoipa::path(
    get,
    path = "/v1/admin/{resource}",
    params(
        ("resource" = String, Path, description = "Admin data resource"),
        ("target_env_id" = String, Query, description = "Target env (RLS context)")
    ),
    responses(
        (status = 200, description = "The resource's rows for the RFW table", body = serde_json::Value),
        (status = 403, description = "Caller lacks a flow read/admin scope"),
        (status = 404, description = "Unknown resource"),
    ),
    security(("sentin8:flow:read" = [])),
    tag = "admin-ui",
)]
pub async fn get_admin_data(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TargetEnvQuery>,
    Path(resource): Path<String>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    let _a = crate::auth::require_any_scope(
        &state.config,
        &headers,
        &[
            "sentin8:flow:read",
            "sentin8:flow:write",
            "sentin8:sys:admin",
            "sentin8:sys:org:admin",
            "sentin8:sys:env:admin",
        ],
    )
    .await?;
    let env_id = q
        .target_env_id
        .as_deref()
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .ok_or_else(|| {
            (
                axum::http::StatusCode::BAD_REQUEST,
                "missing target_env_id".into(),
            )
        })?;
    // RLS: `workflows` is FORCE-RLS'd on (context_initialized, env_id) and
    // set_config(…, true) is transaction-local - the context + queries run
    // inside one explicit transaction on the same connection (flow DESIGN
    // §6.4). A bare-pool read runs context-less and FORCE-RLS hides every
    // row (UI table rendered "No data." while the row existed).
    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    sqlx::query("SELECT sentin8_set_tenant_context($1)")
        .bind(env_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let rows: Vec<serde_json::Value> = match resource.as_str() {
        "workflows" => sqlx::query("SELECT id::text, domain, logic_id, state, current_step FROM workflows WHERE env_id = $1 ORDER BY created_at DESC LIMIT 100")
            .bind(env_id)
            .fetch_all(&mut *tx).await
            .map(|rows| rows.iter().map(|row| serde_json::json!({
                "id": row.get::<Option<String>, &str>("id").unwrap_or_default(),
                "name": row.get::<Option<String>, &str>("domain").unwrap_or_default(),
                "logic_id": row.get::<Option<String>, &str>("logic_id").unwrap_or_default(),
                "status": row.get::<Option<String>, &str>("state").unwrap_or_default(),
                "current_step": row.get::<Option<String>, &str>("current_step").unwrap_or_default(),
            })).collect()).unwrap_or_default(),
        "steps" => {
 // Steps are embedded in the workflow's step_results JSONB array.
 // Query workflows and unnest step_results into individual rows.
            sqlx::query("SELECT id::text as workflow_id, domain, state, current_step, step_results FROM workflows WHERE env_id = $1 ORDER BY created_at DESC LIMIT 50")
                .bind(env_id)
                .fetch_all(&mut *tx).await
                .map(|rows| {
                    let mut steps: Vec<serde_json::Value> = Vec::new();
                    for row in &rows {
                        let workflow_id = row.get::<Option<String>, &str>("workflow_id").unwrap_or_default();
                        let state = row.get::<Option<String>, &str>("state").unwrap_or_default();
                        let step_results: Option<serde_json::Value> = row.try_get("step_results").ok();
                        if let Some(serde_json::Value::Array(arr)) = step_results {
                            for step in arr {
                                if let Some(obj) = step.as_object() {
                                    steps.push(serde_json::json!({
                                        "id": obj.get("step_id").or(obj.get("id")).and_then(|v| v.as_str()).unwrap_or(""),
                                        "workflow_id": workflow_id,
                                        "step_name": obj.get("step_name").or(obj.get("name")).and_then(|v| v.as_str()).unwrap_or(""),
                                        "status": obj.get("status").and_then(|v| v.as_str()).unwrap_or(&state),
                                        "started_at": obj.get("started_at").and_then(|v| v.as_str()).unwrap_or(""),
                                        "completed_at": obj.get("completed_at").and_then(|v| v.as_str()).unwrap_or(""),
                                    }));
                                }
                            }
                        }
                    }
                    steps
                }).unwrap_or_default()
        }
        _ => vec![],
    };
    Ok(Json(serde_json::Value::Array(rows)))
}

fn col(key: &str, label: &str) -> WidgetDataTableColumn {
    WidgetDataTableColumn {
        key: key.into(),
        label: label.into(),
        sortable: false,
        width: None,
        format: None,
    }
}
fn cols(key: &str, label: &str) -> WidgetDataTableColumn {
    WidgetDataTableColumn {
        key: key.into(),
        label: label.into(),
        sortable: true,
        width: None,
        format: None,
    }
}
#[allow(dead_code)]
fn table(columns: Vec<WidgetDataTableColumn>, endpoint: &str) -> RemoteWidget {
    RemoteWidget::DataTable(WidgetDataTable {
        columns,
        data_endpoint: endpoint.into(),
        sortable: true,
        filterable: true,
        page_size: 20,
        empty_message: Some("No data.".into()),
        row_link: None,
        row_actions: vec![],
    })
}
fn table_with_actions(
    columns: Vec<WidgetDataTableColumn>,
    endpoint: &str,
    row_actions: Vec<WidgetButton>,
) -> RemoteWidget {
    RemoteWidget::DataTable(WidgetDataTable {
        columns,
        data_endpoint: endpoint.into(),
        sortable: true,
        filterable: true,
        page_size: 20,
        empty_message: Some("No data.".into()),
        row_link: None,
        row_actions,
    })
}
#[allow(dead_code)]
fn row_action(label: &str, route: &str) -> WidgetButton {
    WidgetButton {
        label: label.into(),
        button_type: ButtonType::Link,
        route: Some(route.into()),
        endpoint: None,
        icon: None,
    }
}
fn row_action_post(label: &str, endpoint: &str) -> WidgetButton {
    WidgetButton {
        label: label.into(),
        button_type: ButtonType::Primary,
        route: None,
        endpoint: Some(endpoint.into()),
        icon: None,
    }
}
fn card(title: &str, body: &str) -> RemoteWidget {
    RemoteWidget::Card(WidgetCard {
        title: Some(title.into()),
        subtitle: None,
        children: vec![RemoteWidget::Text(WidgetText {
            text: body.into(),
            text_type: WidgetTextType::Body,
            align: TextAlign::Left,
            max_lines: None,
        })],
        actions: vec![],
    })
}
#[allow(dead_code)]
fn link(label: &str, route: &str) -> AdminUiAction {
    AdminUiAction {
        label: label.into(),
        action_type: AdminUiActionType::Link,
        route: Some(route.into()),
        endpoint: None,
        icon: Some("add".into()),
    }
}
