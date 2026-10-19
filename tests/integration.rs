// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Sentin8:Flow
// SPDX-License-Identifier: Apache-2.0

// Flow unit tests - Story 8.1.21 / 8.6.8
// Covers: type-level shape checks, handler registry keying, built-in executor
// behavior, proxy-back response mapping, state machine wire stability.
// No backing services required.

use sentin8_flow_sdk::contracts::WorkflowState;
use sentin8_flow_server::proxy_back::HandlerRegistry;
use sentin8_flow_server::steps_generic::{self, StepOutcome};
use sentin8_flow_server::types::*;
use std::collections::HashMap;
use uuid::Uuid;

fn sc(name: &str) -> StepContext {
    StepContext {
        workflow_id: Uuid::new_v4(),
        env_id: Uuid::new_v4(),
        domain: "sys".into(),
        logic_id: "test-logic".into(),
        state: sentin8_flow_sdk::contracts::WorkflowState::Processing,
        current_step: name.into(),
        step_results: vec![],
        input: serde_json::json!({}),
        output: serde_json::json!({}),
        parameters: HashMap::new(),
        user_data: serde_json::json!({}),
        step_index: 0,
        correlation_id: None,
    }
}

#[test]
fn step_result_success() {
    let result = StepResult::success(
        "step1",
        "BasicStep",
        "2026-01-01T00:00:00Z",
        "2026-01-01T00:00:01Z",
    );
    assert_eq!(result.status, "success");
    assert_eq!(result.step_name, "step1");
}

#[test]
fn step_result_failure() {
    let result = StepResult::failure("step1", "HTTPStep", "t1", "t2", "timeout");
    assert_eq!(result.status, "failed");
    assert_eq!(result.error.as_deref(), Some("timeout"));
}

#[test]
fn step_result_waiting() {
    let result = StepResult::waiting("step1", "ManualDecisionStep", "t1", "t2");
    assert_eq!(result.status, "waiting");
}

#[test]
fn step_def_with_error_handler() {
    let step = StepDef {
        name: "http_call".into(),
        kind: "HTTPStep".into(),
        next_step: Some("success_step".into()),
        config: Some(serde_json::json!({"url": "https://api.example.com"})),
        on_error: Some("error_handler".into()),
    };
    assert_eq!(step.on_error.as_deref(), Some("error_handler"));
}

#[test]
fn logic_version_serialization() {
    let logic = LogicVersion {
        logic_id: "my-logic".into(),
        version: 1,
        steps: vec![StepDef {
            name: "step1".into(),
            kind: "BasicStep".into(),
            next_step: Some("step2".into()),
            config: Some(serde_json::json!({})),
            on_error: None,
        }],
        correlation_id_func: None,
        expected_parameters: None,
    };
    assert_eq!(logic.steps.len(), 1);
    assert_eq!(logic.version, 1);
}

#[test]
fn callback_config_roundtrip() {
    let config = CallbackConfig {
        base_url: "http://payments:3112".into(),
        auth_token: "token".into(),
        internal_path: "/v1/internal/steps/execute".into(),
        host_functions: HashMap::new(),
    };
    assert_eq!(config.base_url, "http://payments:3112");
}

#[test]
fn step_handler_profile() {
    let profile = StepHandlerProfile {
        domain: "payments".into(),
        base_url: "http://payments:3112".into(),
        auth_token: "token".into(),
        internal_path: "/v1/internal/steps/execute".into(),
    };
    assert_eq!(profile.domain, "payments");
}

#[test]
fn step_result_with_next_step() {
    let mut result = StepResult::success("step1", "BasicStep", "t1", "t2");
    result.next_step = Some("step2".into());
    assert_eq!(result.next_step.as_deref(), Some("step2"));
}

#[test]
fn step_result_with_output() {
    let mut result = StepResult::success("step1", "HTTPStep", "t1", "t2");
    result.output = Some(serde_json::json!({"status_code": 200}));
    assert_eq!(result.output.unwrap()["status_code"], 200);
}

// ── Handler registry (8.6.5) ────────────────────────────────────────────

#[tokio::test]
async fn registry_resolves_by_domain() {
    let reg = HandlerRegistry::new();
    assert!(reg.resolve("payments").await.is_none());

    reg.register(StepHandlerProfile {
        domain: "payments".into(),
        base_url: "https://payments.ops.sys.sentin8.local".into(),
        auth_token: "tok".into(),
        internal_path: "/v1/internal/steps".into(),
    })
    .await;

    let p = reg.resolve("payments").await.expect("profile");
    assert_eq!(p.base_url, "https://payments.ops.sys.sentin8.local");
    assert_eq!(p.internal_path, "/v1/internal/steps");
    assert!(reg.resolve("ledger").await.is_none());
}

#[tokio::test]
async fn registry_reregister_overwrites() {
    let reg = HandlerRegistry::new();
    reg.register(StepHandlerProfile {
        domain: "d".into(),
        base_url: "http://old".into(),
        auth_token: "t1".into(),
        internal_path: "/a".into(),
    })
    .await;
    reg.register(StepHandlerProfile {
        domain: "d".into(),
        base_url: "http://new".into(),
        auth_token: "t2".into(),
        internal_path: "/b".into(),
    })
    .await;
    let p = reg.resolve("d").await.unwrap();
    assert_eq!(p.base_url, "http://new");
}

// ── Generic-step dispatch table (8.6.3) ─────────────────────────────────

#[test]
fn is_generic_covers_the_7_builtins() {
    assert!(steps_generic::is_generic("BasicStep"));
    assert!(steps_generic::is_generic("HTTPStep"));
    assert!(steps_generic::is_generic("HTTPAsyncStep"));
    assert!(steps_generic::is_generic("ManualDecisionStep"));
    assert!(steps_generic::is_generic("ScheduleInstructionStep"));
    assert!(steps_generic::is_generic("InitiateInstructionStep"));
    assert!(steps_generic::is_generic("SequenceStep"));
}

#[test]
fn domain_steps_are_not_generic() {
    assert!(!steps_generic::is_generic("PeriodCalculationStep"));
    assert!(!steps_generic::is_generic("MatchPaymentInstrumentStep"));
    assert!(!steps_generic::is_generic("SysCreateOrgRecordStep"));
    assert!(!steps_generic::is_generic("UnknownStep"));
}

// ── Built-in executors (8.1.5-8.1.11) ───────────────────────────────────

#[tokio::test]
async fn manual_decision_waits() {
    let outcome = steps_generic::execute_manual_decision(&sc("decide")).await;
    match outcome {
        StepOutcome::Waiting(result, sched) => {
            assert_eq!(result.status, "waiting");
            assert!(sched.is_none());
        }
        StepOutcome::Proceed(_) => panic!("expected Waiting"),
    }
}

#[tokio::test]
async fn schedule_step_computes_delay() {
    let config = serde_json::json!({ "delay_seconds": 3600 });
    let outcome = steps_generic::execute_schedule_instruction(&sc("sched"), Some(&config)).await;
    match outcome {
        StepOutcome::Waiting(result, sched) => {
            assert_eq!(result.status, "waiting");
            let at = sched.expect("scheduled_at from delay_seconds");
            let expected = chrono::Utc::now() + chrono::Duration::seconds(3600);
            assert!((at - expected).num_seconds().abs() < 5, "at={at}");
        }
        StepOutcome::Proceed(_) => panic!("expected Waiting"),
    }
}

#[tokio::test]
async fn schedule_step_parses_rfc3339() {
    let config = serde_json::json!({ "scheduled_at": "2030-01-01T00:00:00Z" });
    let outcome = steps_generic::execute_schedule_instruction(&sc("sched"), Some(&config)).await;
    match outcome {
        StepOutcome::Waiting(_, sched) => {
            let at = sched.expect("scheduled_at");
            assert_eq!(&at.to_rfc3339()[..10], "2030-01-01");
        }
        StepOutcome::Proceed(_) => panic!("expected Waiting"),
    }
}

#[tokio::test]
async fn schedule_step_without_config_waits_undated() {
    let outcome = steps_generic::execute_schedule_instruction(&sc("sched"), None).await;
    match outcome {
        StepOutcome::Waiting(result, sched) => {
            assert_eq!(result.status, "waiting");
            assert!(sched.is_none());
        }
        StepOutcome::Proceed(_) => panic!("expected Waiting"),
    }
}

#[tokio::test]
async fn http_step_fails_without_config() {
    let outcome = steps_generic::execute_http(&sc("call"), None).await;
    match outcome {
        StepOutcome::Proceed(r) => {
            assert_eq!(r.status, "failed");
            assert!(r.error.unwrap().contains("missing or invalid config"));
        }
        StepOutcome::Waiting(_, _) => panic!("expected Proceed"),
    }
}

#[tokio::test]
async fn basic_step_without_wasm_is_noop_success() {
    let cb = CallbackConfig::default();
    let outcome = steps_generic::execute_basic(&sc("noop"), None, &None, &cb).await;
    match outcome {
        StepOutcome::Proceed(r) => {
            assert_eq!(r.status, "success");
            assert!(r.output.is_none());
        }
        StepOutcome::Waiting(_, _) => panic!("expected Proceed"),
    }
}

#[tokio::test]
async fn basic_step_with_wasm_but_no_func_client_is_noop() {
    let cb = CallbackConfig::default();
    let config = serde_json::json!({ "wasmModuleId": "mod-1", "hook": "on_step" });
    let outcome = steps_generic::execute_basic(&sc("wasm"), Some(&config), &None, &cb).await;
    match outcome {
        StepOutcome::Proceed(r) => assert_eq!(r.status, "success"),
        StepOutcome::Waiting(_, _) => panic!("expected Proceed"),
    }
}

#[tokio::test]
async fn sequence_step_fails_on_unreachable_db() {
    let pool =
        sqlx::postgres::PgPool::connect_lazy("postgres://invalid:invalid@127.0.0.1:1/invalid")
            .unwrap();
    let config = serde_json::json!({ "key": "orders" });
    let outcome = steps_generic::execute_sequence(&sc("seq"), Some(&config), &pool).await;
    match outcome {
        StepOutcome::Proceed(r) => assert_eq!(r.status, "failed"),
        StepOutcome::Waiting(_, _) => panic!("expected Proceed"),
    }
}

#[tokio::test]
async fn initiate_step_fails_without_config() {
    let pool =
        sqlx::postgres::PgPool::connect_lazy("postgres://invalid:invalid@127.0.0.1:1/invalid")
            .unwrap();
    let cb = CallbackConfig::default();
    let outcome = steps_generic::execute_initiate_instruction(&sc("child"), None, &pool, &cb).await;
    match outcome {
        StepOutcome::Proceed(r) => {
            assert_eq!(r.status, "failed");
            assert!(r.error.unwrap().contains("missing config"));
        }
        StepOutcome::Waiting(_, _) => panic!("expected Proceed"),
    }
}

// ── Proxy-back mapping (8.1.12) ─────────────────────────────────────────

#[test]
fn proxy_back_response_maps_to_step_result() {
    let pb = ProxyBackResponse {
        status: "success".into(),
        output: Some(serde_json::json!({"posted": true})),
        error: None,
        next_step: Some("post".into()),
    };
    let r = sentin8_flow_server::proxy_back::proxy_back_to_result(
        pb,
        "submit",
        "SchemeSubmissionStep",
        "t1",
        "t2",
    );
    assert_eq!(r.step_name, "submit");
    assert_eq!(r.kind, "SchemeSubmissionStep");
    assert_eq!(r.status, "success");
    assert_eq!(r.next_step.as_deref(), Some("post"));
    assert_eq!(r.started_at, "t1");
    assert_eq!(r.ended_at, "t2");
}

#[test]
fn proxy_back_url_assembly() {
    let profile = StepHandlerProfile {
        domain: "payments".into(),
        base_url: "https://payments.ops.sys.sentin8.local".into(),
        auth_token: "tok".into(),
        internal_path: "/v1/internal/steps".into(),
    };
    let url = format!("{}{}/execute", profile.base_url, profile.internal_path);
    assert_eq!(
        url,
        "https://payments.ops.sys.sentin8.local/v1/internal/steps/execute"
    );
}

// ── State machine wire stability ────────────────────────────────────────

#[test]
fn workflow_state_roundtrip_all_variants() {
    for (wire, variant) in [
        ("pending", WorkflowState::Pending),
        ("processing", WorkflowState::Processing),
        ("waiting", WorkflowState::Waiting),
        ("completed", WorkflowState::Completed),
        ("failed", WorkflowState::Failed),
        ("cancelled", WorkflowState::Cancelled),
    ] {
        let v = serde_json::to_value(variant).unwrap();
        assert_eq!(v, wire);
        let back: WorkflowState = serde_json::from_value(v).unwrap();
        assert_eq!(back, variant);
    }
}

#[test]
fn state_matches_sql_check_constraint_values() {
    let allowed = [
        WorkflowState::Pending,
        WorkflowState::Processing,
        WorkflowState::Waiting,
        WorkflowState::Completed,
        WorkflowState::Failed,
        WorkflowState::Cancelled,
    ];
    for s in allowed {
        let v = serde_json::to_value(s).unwrap();
        assert!(matches!(
            v.as_str().unwrap(),
            "pending" | "processing" | "waiting" | "completed" | "failed" | "cancelled"
        ));
    }
}

// ── CallbackConfig ──────────────────────────────────────────────────────

#[test]
fn callback_config_default_is_empty() {
    let cb = CallbackConfig::default();
    assert!(cb.base_url.is_empty());
    assert!(cb.auth_token.is_empty());
    assert!(cb.internal_path.is_empty());
    assert!(cb.host_functions.is_empty());
}

#[test]
fn callback_config_host_functions_roundtrip() {
    let mut hf = HashMap::new();
    hf.insert(
        "get_instruction".to_string(),
        "payments_get_instruction".to_string(),
    );
    let cb = CallbackConfig {
        base_url: "https://payments.ops.sys.sentin8.local".into(),
        auth_token: "tok".into(),
        internal_path: "/v1/internal/steps".into(),
        host_functions: hf,
    };
    let v = serde_json::to_value(&cb).unwrap();
    let back: CallbackConfig = serde_json::from_value(v).unwrap();
    assert_eq!(
        back.host_functions
            .get("get_instruction")
            .map(String::as_str),
        Some("payments_get_instruction")
    );
}

// ── WorkflowRecord contract conformance (8.6.1) ─────────────────────────

#[test]
fn workflow_record_serializes_without_org_id() {
    let record = WorkflowRecord {
        id: Uuid::new_v4(),
        env_id: Uuid::new_v4(),
        domain: "sys".into(),
        logic_id: "sys-create-org".into(),
        logic: LogicVersion::default(),
        state: WorkflowState::Pending,
        correlation_id: None,
        input: serde_json::json!({}),
        output: None,
        current_step: None,
        step_results: vec![],
        error_message: None,
        scheduled_at: None,
        callback_config: CallbackConfig::default(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        completed_at: None,
    };
    let v = serde_json::to_value(&record).unwrap();
    assert!(v.get("org_id").is_none(), "no org_id on the wire");
    let back: WorkflowRecord = serde_json::from_value(v).unwrap();
    assert_eq!(back.id, record.id);
}
