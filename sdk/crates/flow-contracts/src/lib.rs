// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Sentin8:Flow
// SPDX-License-Identifier: Apache-2.0

//! # flow-contracts
//!
//! Rust types for the Flow standalone workflow execution service.

#![deny(missing_docs)]

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// Lifecycle state of a workflow instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum WorkflowState {
    /// Created, not yet started.
    Pending,
    /// Currently executing a step.
    Processing,
    /// Blocked on an external decision or timer.
    Waiting,
    /// Finished successfully.
    Completed,
    /// Terminated by an unrecoverable step error.
    Failed,
    /// Canceled by an operator or the owning service.
    Cancelled,
}

/// WorkflowRecord - the persisted workflow state as returned by Sentin8:Flow.
/// Mirrors the server's `flow.workflows` table (no `org_id` - RLS is
/// `env_id`-scoped).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct WorkflowRecord {
    /// Workflow instance id.
    pub id: Uuid,
    /// Owning environment (the tenant key).
    pub env_id: Uuid,
    /// Owning business domain (e.g. `ledger`).
    pub domain: String,
    /// The logic definition this instance runs.
    pub logic_id: String,
    /// Current lifecycle state.
    pub state: WorkflowState,
    /// Correlation id linking the instance to its triggering event.
    #[serde(default)]
    pub correlation_id: Option<String>,
    /// Input payload the instance was started with.
    #[serde(default)]
    pub input: serde_json::Value,
    /// Final output, set when the instance completes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<serde_json::Value>,
    /// The step currently executing or waiting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_step: Option<String>,
    /// Per-step execution results, in execution order.
    #[serde(default)]
    pub step_results: Vec<serde_json::Value>,
    /// Error detail when the instance failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    /// When a deferred start is scheduled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduled_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Proxy-back callback configuration for domain steps.
    #[serde(default)]
    pub callback_config: serde_json::Value,
    /// Creation time (UTC).
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Last update time (UTC).
    #[serde(default)]
    pub updated_at: chrono::DateTime<chrono::Utc>,
    /// Completion time, set when the instance completes (UTC).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn workflow_state_serializes_lowercase() {
        for (s, wire) in [
            (WorkflowState::Pending, "pending"),
            (WorkflowState::Processing, "processing"),
            (WorkflowState::Waiting, "waiting"),
            (WorkflowState::Completed, "completed"),
            (WorkflowState::Failed, "failed"),
            (WorkflowState::Cancelled, "cancelled"),
        ] {
            let v = serde_json::to_value(s).unwrap();
            assert_eq!(v, wire, "wire name for {s:?}");
            let back: WorkflowState = serde_json::from_value(v).unwrap();
            assert_eq!(back, s);
        }
    }

    #[test]
    fn workflow_record_roundtrip() {
        let w = WorkflowRecord {
            id: uuid::Uuid::new_v4(),
            env_id: uuid::Uuid::new_v4(),
            domain: "ledger".into(),
            logic_id: "statement-regen".into(),
            state: WorkflowState::Waiting,
            correlation_id: Some("corr-1".into()),
            input: json!({"account_id": "a-1"}),
            output: Some(json!({"posted": true})),
            current_step: Some("post".into()),
            step_results: vec![json!({"step_name": "post", "status": "waiting"})],
            error_message: None,
            scheduled_at: None,
            callback_config: json!({}),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            completed_at: None,
        };
        let v = serde_json::to_value(&w).unwrap();
        assert_eq!(v["state"], "waiting");
        assert_eq!(v["output"]["posted"], true);
        assert!(v.get("org_id").is_none());
        let back: WorkflowRecord = serde_json::from_value(v).unwrap();
        assert_eq!(back.id, w.id);
        assert_eq!(back.state, WorkflowState::Waiting);
        assert_eq!(back.correlation_id.as_deref(), Some("corr-1"));
    }

    #[test]
    fn workflow_record_decodes_minimal_server_response() {
        let v = json!({
            "id": uuid::Uuid::new_v4(),
            "env_id": uuid::Uuid::new_v4(),
            "domain": "sys",
            "logic_id": "sys-create-org",
            "state": "pending",
            "created_at": "2026-01-01T00:00:00Z"
        });
        let w: WorkflowRecord = serde_json::from_value(v).unwrap();
        assert_eq!(w.state, WorkflowState::Pending);
        assert!(w.output.is_none());
        assert!(w.step_results.is_empty());
    }

    #[test]
    fn workflow_record_output_omitted_when_none() {
        let w = WorkflowRecord {
            id: uuid::Uuid::new_v4(),
            env_id: uuid::Uuid::new_v4(),
            domain: "d".into(),
            logic_id: "l".into(),
            state: WorkflowState::Pending,
            correlation_id: None,
            input: json!({}),
            output: None,
            current_step: None,
            step_results: vec![],
            error_message: None,
            scheduled_at: None,
            callback_config: json!({}),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            completed_at: None,
        };
        let v = serde_json::to_value(&w).unwrap();
        assert!(v.get("output").is_none());
        let back: WorkflowRecord = serde_json::from_value(v).unwrap();
        assert!(back.output.is_none());
    }
}
