// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Sentin8:Flow
// SPDX-License-Identifier: Apache-2.0

//! Step execution engine - sequential loop, state machine, branching.

use sentin8_flow_sdk::contracts::WorkflowState;
use uuid::Uuid;

use crate::proxy_back::{self, HandlerRegistry};
use crate::steps_generic::{self, StepOutcome};
use crate::store;
use crate::types::{
    CallbackConfig, ProxyBackRequest, StepContext, StepDef, StepHandlerProfile, StepResult,
    WorkflowRecord,
};

/// Default deadline (seconds) for a `waiting` state without an explicit
/// `scheduled_at` (flow DESIGN §6): the workflow cannot park forever -
/// the due-index + Cron resume path expires it as Failed after this
/// window.
const DEFAULT_WAIT_DEADLINE_SECS: i64 = 7 * 24 * 3600;

/// The engine type.
pub struct Engine {
    /// The pool.
    pub pool: sqlx::PgPool,
    /// The CERT-PRESENTING http transport (, auth DESIGN
    /// §4.3f): presents the `sentin8-flow-proxy-back` workload certificate on
    /// every call - proxy-back targets authenticate it at the TLS
    /// handshake; no bearer token exists on this path.
    pub http: sentin8::middleware::http_client::Sentin8HttpClient,
    /// The registry.
    pub registry: HandlerRegistry,
    /// The func client.
    pub func_client: Option<sentin8_func_sdk::client::FuncClient>,
}

impl Engine {
    /// Execute a workflow from its current step. Updates the record in the DB
    /// and emits outbox events for state transitions.
    /// Execute it.
    pub async fn execute(
        &self,
        env_id: Uuid,
        mut record: WorkflowRecord,
        logic: &crate::types::LogicVersion,
    ) -> WorkflowRecord {
        // Transition to processing.
        record = match store::update_workflow(
            &self.pool,
            env_id,
            record.id,
            WorkflowState::Processing,
            record.current_step.as_deref(),
            &record.step_results,
            record.output.as_ref(),
            record.error_message.as_deref(),
            None,
        )
        .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(error = %e, "failed to set processing state");
                return record;
            }
        };
        let _ = store::insert_outbox(
            &self.pool,
            env_id,
            "sentin8:flow:workflow:started",
            &serde_json::json!({"workflow_id" : record.id, "logic_id" : record.logic_id}),
        )
        .await;

        // Determine starting step index.
        let start_idx = if let Some(cur) = &record.current_step {
            logic.steps.iter().position(|s| &s.name == cur).unwrap_or(0)
        } else {
            0
        };

        let mut step_results = record.step_results.clone();
        let mut current_step: Option<String> = None;
        let mut output = record.output.clone();
        let mut error_msg: Option<String> = None;
        let mut scheduled_at: Option<chrono::DateTime<chrono::Utc>> = None;
        let mut final_state = WorkflowState::Completed;
        // Branching support: a step may redirect control flow to a named step
        // (via a failed `branch:<target>` onError, or an explicit `next_step`
        // from either the step def or the step result). The engine jumps to
        // that step's index on the next loop iteration instead of proceeding
        // sequentially.
        let mut jump_to: Option<String> = None;

        let mut i = 0usize;
        while i < logic.steps.len() {
            if jump_to.is_none() && i < start_idx {
                i += 1;
                continue;
            }
            // A pending jump overrides sequential order.
            if let Some(target) = jump_to.take() {
                let Some(idx) = logic.steps.iter().position(|s| s.name == target) else {
                    // Unknown branch target - fail the workflow.
                    error_msg = Some(format!("branch target not found: {target}"));
                    final_state = WorkflowState::Failed;
                    break;
                };
                if idx < i && idx < start_idx {
                    // Jumping backwards before the resume point is allowed -
                    // the engine re-executes from that step.
                }
                i = idx;
            }
            let step = &logic.steps[i];
            current_step = Some(step.name.clone());
            let sc = StepContext {
                workflow_id: record.id,
                env_id,
                domain: record.domain.clone(),
                logic_id: record.logic_id.clone(),
                state: WorkflowState::Processing,
                current_step: step.name.clone(),
                step_results: step_results.clone(),
                input: record.input.clone(),
                output: output.clone().unwrap_or(serde_json::Value::Null),
                parameters: std::collections::HashMap::new(),
                user_data: record.input.clone(),
                step_index: i,
                correlation_id: record.correlation_id.clone(),
            };

            let outcome = if steps_generic::is_generic(&step.kind) {
                steps_generic::execute_generic(
                    &sc,
                    &step.kind,
                    step.config.as_ref(),
                    &self.pool,
                    &self.func_client,
                    &record.callback_config,
                )
                .await
            } else {
                // Domain step - proxy-back.
                self.proxy_back_step(&sc, step, &record.callback_config)
                    .await
            };

            match outcome {
                StepOutcome::Proceed(result) => {
                    let next = determine_next(step, &result);
                    step_results.push(result.clone());
                    // Update output if the step produced one.
                    if let Some(out) = &result.output {
                        output = Some(out.clone());
                    }
                    if result.status == "failed" {
                        let on_error = step.on_error.as_deref().unwrap_or("fail");
                        match on_error {
                            "continue" => {
                                // proceed to next step
                                i += 1;
                            }
                            "fail" => {
                                error_msg = result.error.clone();
                                final_state = WorkflowState::Failed;
                                current_step = Some(step.name.clone());
                                break;
                            }
                            branch if branch.starts_with("branch:") => {
                                let target = branch.trim_start_matches("branch:");
                                if logic.steps.iter().any(|s| s.name == target) {
                                    jump_to = Some(target.to_string());
                                } else {
                                    error_msg = Some(format!("branch target not found: {target}"));
                                    final_state = WorkflowState::Failed;
                                }
                                if final_state == WorkflowState::Failed {
                                    break;
                                }
                            }
                            _ => {
                                error_msg = result.error.clone();
                                final_state = WorkflowState::Failed;
                                break;
                            }
                        }
                    } else {
                        // Honor explicit nextStep override (step def or result).
                        if let Some(ns) = &next {
                            if ns != &step.name && logic.steps.iter().any(|s| &s.name == ns) {
                                jump_to = Some(ns.clone());
                            }
                        }
                        i += 1;
                    }
                }
                StepOutcome::Waiting(result, sched) => {
                    step_results.push(result);
                    // Deadline for undated waits (flow DESIGN §6): a wait
                    // without an explicit `scheduled_at` gets a default
                    // deadline so the workflow cannot park forever - the
                    // due-index + Cron resume path expires it as Failed.
                    scheduled_at = match sched {
                        Some(s) => Some(s),
                        None => Some(
                            chrono::Utc::now()
                                + chrono::Duration::seconds(DEFAULT_WAIT_DEADLINE_SECS),
                        ),
                    };
                    final_state = WorkflowState::Waiting;
                    current_step = Some(step.name.clone());
                    break;
                }
            }
        }

        // Persist final state.
        let updated = store::update_workflow(
            &self.pool,
            env_id,
            record.id,
            final_state,
            current_step.as_deref(),
            &step_results,
            output.as_ref(),
            error_msg.as_deref(),
            scheduled_at,
        )
        .await;
        match updated {
            Ok(r) => {
                let evt = match final_state {
                    WorkflowState::Completed => "sentin8:flow:workflow:completed",
                    WorkflowState::Failed => "sentin8:flow:workflow:failed",
                    WorkflowState::Waiting => "sentin8:flow:workflow:waiting",
                    _ => "sentin8:flow:workflow:updated",
                };
                let _ = store::insert_outbox(&self.pool, env_id, evt, &serde_json::json!({"workflow_id" : r.id, "state": format!("{:?}", final_state)})).await;
                r
            }
            Err(e) => {
                tracing::error!(error = %e, "failed to persist final state");
                record
            }
        }
    }

    /// Proxy-call the domain service for a domain-specific step.
    async fn proxy_back_step(
        &self,
        sc: &StepContext,
        step: &StepDef,
        callback: &CallbackConfig,
    ) -> StepOutcome {
        let started = chrono::Utc::now().to_rfc3339();
        // Resolve the handler profile by the workflow's domain; fall back to
        // the per-workflow callback config when no registered profile matches.
        let profile = self
            .registry
            .resolve(&sc.domain)
            .await
            .unwrap_or(StepHandlerProfile {
                domain: sc.domain.clone(),
                base_url: callback.base_url.clone(),
                auth_token: callback.auth_token.clone(),
                internal_path: callback.internal_path.clone(),
            });
        let req = ProxyBackRequest {
            step_kind: step.kind.clone(),
            workflow_id: sc.workflow_id,
            env_id: sc.env_id,
            step_name: step.name.clone(),
            input: sc.input.clone(),
            user_data: sc.user_data.clone(),
            step_config: step.config.clone(),
        };
        let ended = chrono::Utc::now().to_rfc3339();
        match proxy_back::proxy_back(&self.http, &profile, req).await {
            Ok(pb) => StepOutcome::Proceed(proxy_back::proxy_back_to_result(
                pb, &step.name, &step.kind, &started, &ended,
            )),
            Err(e) => StepOutcome::Proceed(StepResult::failure(
                &step.name,
                &step.kind,
                &started,
                &ended,
                &e.to_string(),
            )),
        }
    }
}

fn determine_next(step: &StepDef, result: &StepResult) -> Option<String> {
    if let Some(ns) = &result.next_step {
        return Some(ns.clone());
    }
    step.next_step.clone()
}
