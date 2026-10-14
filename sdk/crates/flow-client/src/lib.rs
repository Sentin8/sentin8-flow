// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Sentin8:Flow
// SPDX-License-Identifier: Apache-2.0

//! # flow-client
//!
//! Rust HTTP client for the Flow standalone workflow execution service.

#![deny(missing_docs)]

use sentin8_flow_contracts::WorkflowRecord;
use sentin8_middleware::http_client::Sentin8HttpClient;
use thiserror::Error;

/// Errors returned by the Flow client.
#[derive(Debug, Error)]
pub enum FlowError {
    /// The HTTP request failed or returned a non-success status.
    #[error("Flow HTTP error: {0}")]
    Http(String),
    /// The response body could not be decoded.
    #[error("Flow decode error: {0}")]
    Decode(String),
}

/// Percent-encode a query-parameter value (RFC 3986 unreserved set kept
/// literal; everything else escaped) — the workspace's shared no-dep
/// encoding convention (BFF `handlers::urlencoding` pattern).
fn percent_encode_query(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push_str(&format!("%{b:02X}"));
            }
        }
    }
    out
}

/// The Flow client — **certificate-presenting** (, auth DESIGN
/// §4.3f): every call presents the workload's Acme-issued client
/// certificate in the TLS handshake; Flow's TLS layer authenticates it.
/// No bearer token exists on this path. Cheap to clone.
#[derive(Clone)]
pub struct FlowClient {
    base_url: String,
    http: Sentin8HttpClient,
}

impl FlowClient {
    /// Build a certificate-presenting FlowClient. The transport presents
    /// `cert_path`/`key_path` (the workload's Acme-issued identity) on
    /// every call and trusts only `ca_cert_path`.
    ///
    /// # Errors
    /// [`FlowError::Http`] when the mTLS transport cannot be built from
    /// the identity paths — a missing certificate identity is a
    /// provisioning state, surfaced loud.
    pub fn with_workload_identity(
        base_url: impl Into<String>,
        ca_cert_path: &str,
        cert_path: &str,
        key_path: &str,
    ) -> Result<Self, FlowError> {
        let mtls = sentin8_middleware::mtls::MtlsConfig::new(ca_cert_path, cert_path, key_path);
        let http = sentin8_middleware::mtls::build_mtls_client(&mtls)
            .map(Sentin8HttpClient::from_client)
            .map_err(|e| FlowError::Http(e.to_string()))?;
        Ok(Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            http,
        })
    }

    /// Plain-HTTP DEV FALLBACK constructor — no TLS identity (Flow
    /// rejects it loudly at the authorization gate). Use ONLY for the
    /// fail-soft boot path when the workload certificate cannot be minted
    /// yet; never for production paths.
    pub fn new_dev_plain(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            http: Sentin8HttpClient::new(),
        }
    }

    /// Wrap a pre-built cert-presenting transport (— for
    /// callers that hold a shared per-process transport (sys's delegation
    /// client builds per call against one boot-installed transport).
    pub fn from_transport(base_url: impl Into<String>, http: Sentin8HttpClient) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            http,
        }
    }

    /// Test-only plain-HTTP constructor — talks to a local test server
    /// with no TLS/identity.
    #[cfg(test)]
    fn plain(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            http: Sentin8HttpClient::new(),
        }
    }

    /// `GET /v1/workflows/{id}` — fetch a workflow instance.
    pub async fn get_workflow(&self, id: &str) -> Result<WorkflowRecord, FlowError> {
        let resp = self
            .http
            .get(format!("{}/v1/workflows/{id}", self.base_url))
            .send()
            .await
            .map_err(|e| FlowError::Http(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(FlowError::Http(format!("status {}", resp.status())));
        }
        resp.json()
            .await
            .map_err(|e| FlowError::Decode(e.to_string()))
    }

    /// `POST /v1/workflows` — create + start workflow.
    pub async fn create_workflow(
        &self,
        req: &CreateWorkflowRequest,
    ) -> Result<WorkflowRecord, FlowError> {
        let resp = self
            .http
            .post(format!("{}/v1/workflows", self.base_url))
            .json(req)
            .send()
            .await
            .map_err(|e| FlowError::Http(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(FlowError::Http(format!("status {}", resp.status())));
        }
        resp.json()
            .await
            .map_err(|e| FlowError::Decode(e.to_string()))
    }

    /// `GET /v1/workflows` — list workflows.
    ///
    /// All filters are optional; `correlation_id` rides the
    /// `idx_flow_workflows_correlation` partial index server-side, so
    /// callers matching a workflow by its correlation (e.g. an
    /// instruction id) should always pass it instead of list-and-scan —
    /// the unfiltered list is `ORDER BY created_at DESC LIMIT 100`.
    pub async fn list_workflows(
        &self,
        domain: Option<&str>,
        state: Option<&str>,
        correlation_id: Option<&str>,
    ) -> Result<Vec<WorkflowRecord>, FlowError> {
        let mut url = format!("{}/v1/workflows", self.base_url);
        let mut sep = '?';
        for (key, value) in [
            ("domain", domain),
            ("state", state),
            ("correlation_id", correlation_id),
        ] {
            if let Some(v) = value {
                url.push(sep);
                url.push_str(key);
                url.push('=');
                url.push_str(&percent_encode_query(v));
                sep = '&';
            }
        }
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| FlowError::Http(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(FlowError::Http(format!("status {}", resp.status())));
        }
        resp.json()
            .await
            .map_err(|e| FlowError::Decode(e.to_string()))
    }

    /// `POST /v1/workflows/{id}/retry`
    pub async fn retry_workflow(&self, id: &str) -> Result<WorkflowRecord, FlowError> {
        let resp = self
            .http
            .post(format!("{}/v1/workflows/{id}/retry", self.base_url))
            .send()
            .await
            .map_err(|e| FlowError::Http(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(FlowError::Http(format!("status {}", resp.status())));
        }
        resp.json()
            .await
            .map_err(|e| FlowError::Decode(e.to_string()))
    }

    /// `POST /v1/workflows/{id}/repair`
    pub async fn repair_workflow(
        &self,
        id: &str,
        rewind_to: usize,
        input: Option<&serde_json::Value>,
    ) -> Result<WorkflowRecord, FlowError> {
        let body = serde_json::json!({ "rewind_to" : rewind_to, "input": input });
        let resp = self
            .http
            .post(format!("{}/v1/workflows/{id}/repair", self.base_url))
            .json(&body)
            .send()
            .await
            .map_err(|e| FlowError::Http(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(FlowError::Http(format!("status {}", resp.status())));
        }
        resp.json()
            .await
            .map_err(|e| FlowError::Decode(e.to_string()))
    }

    /// `POST /v1/workflows/{id}/cancel`
    pub async fn cancel_workflow(&self, id: &str) -> Result<WorkflowRecord, FlowError> {
        let resp = self
            .http
            .post(format!("{}/v1/workflows/{id}/cancel", self.base_url))
            .send()
            .await
            .map_err(|e| FlowError::Http(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(FlowError::Http(format!("status {}", resp.status())));
        }
        resp.json()
            .await
            .map_err(|e| FlowError::Decode(e.to_string()))
    }

    /// `POST /v1/workflows/{id}/resume`
    pub async fn resume_workflow(
        &self,
        id: &str,
        decision: Option<&serde_json::Value>,
    ) -> Result<WorkflowRecord, FlowError> {
        let body = serde_json::json!({ "decision": decision });
        let resp = self
            .http
            .post(format!("{}/v1/workflows/{id}/resume", self.base_url))
            .json(&body)
            .send()
            .await
            .map_err(|e| FlowError::Http(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(FlowError::Http(format!("status {}", resp.status())));
        }
        resp.json()
            .await
            .map_err(|e| FlowError::Decode(e.to_string()))
    }
}

// ── Request types ───────────────────────────────────────────────────────

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A single step in a logic definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepDef {
    /// Step name (unique within the logic).
    pub name: String,
    /// Step executor kind (e.g. `BasicStep`).
    pub kind: String,
    /// The next step in the chain, when the flow is linear.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_step: Option<String>,
    /// Executor-specific configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<serde_json::Value>,
    /// Step to jump to on error, when overriding the default failure path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_error: Option<String>,
}

/// A versioned logic definition (the workflow template).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogicVersion {
    /// Logic identifier.
    pub logic_id: String,
    /// Monotonically increasing version.
    pub version: i32,
    /// The steps of the logic, keyed by `next_step` chaining.
    pub steps: Vec<StepDef>,
    /// Function used to derive the correlation id from the input.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub correlation_id_func: Option<String>,
    /// Input parameter names the logic expects.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_parameters: Option<Vec<String>>,
}

/// Proxy-back configuration: how Flow calls the owning service's
/// internal API for domain steps.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallbackConfig {
    /// Base URL of the owning service.
    pub base_url: String,
    /// Bearer token used for the proxy-back calls.
    pub auth_token: String,
    /// Internal path to POST for each domain step.
    pub internal_path: String,
    /// Step name → host function id mapping.
    #[serde(default)]
    pub host_functions: HashMap<String, String>,
}

/// Request to create and start a workflow instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateWorkflowRequest {
    /// Owning business domain (e.g. `ledger`).
    pub domain: String,
    /// The logic definition (template) to run.
    pub logic: LogicVersion,
    /// Input payload for the first step.
    pub input: serde_json::Value,
    /// Proxy-back callback configuration.
    pub callback_config: CallbackConfig,
    /// Optional correlation id; derived from input when omitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn install_provider() {
        use std::sync::Once;
        static ONCE: Once = Once::new();
        ONCE.call_once(|| {
            let _ = sentin8_tls::crypto::sentin8_pq_provider().install_default();
        });
    }

    /// One-shot HTTP server: replies to one request with `(status, body)`.
    /// Returns the handle whose await yields the raw request text.
    async fn serve(
        status_line: &'static str,
        body: &'static str,
    ) -> (tokio::task::JoinHandle<String>, std::net::SocketAddr) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 8192];
            let n = sock.read(&mut buf).await.unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            let resp = format!(
                "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
            req
        });
        (handle, addr)
    }

    const SAMPLE_WORKFLOW_JSON: &str = r#"{"id":"01960000-0000-7000-8000-000000000001","org_id":"01960000-0000-7000-8000-000000000002","env_id":"01960000-0000-7000-8000-000000000003","domain":"ledger","logic_id":"statement-regen","state":"waiting","input":{},"created_at":"2026-01-01T00:00:00Z"}"#;

    #[tokio::test]
    async fn create_workflow_posts_full_body() {
        install_provider();
        let (handle, addr) = serve("HTTP/1.1 200 OK", SAMPLE_WORKFLOW_JSON).await;
        let c = FlowClient::plain(format!("http://{addr}"));
        let req = CreateWorkflowRequest {
            domain: "ledger".into(),
            logic: LogicVersion {
                logic_id: "statement-regen".into(),
                version: 1,
                steps: vec![StepDef {
                    name: "fetch".into(),
                    kind: "BasicStep".into(),
                    next_step: None,
                    config: Some(serde_json::json!({})),
                    on_error: None,
                }],
                correlation_id_func: None,
                expected_parameters: None,
            },
            input: serde_json::json!({"account_id": "a-1"}),
            callback_config: CallbackConfig {
                base_url: "https://ledger.local".into(),
                auth_token: "cb-tok".into(),
                internal_path: "/v1/internal".into(),
                host_functions: Default::default(),
            },
            correlation_id: Some("corr-1".into()),
        };
        let w = c.create_workflow(&req).await.unwrap();
        assert_eq!(w.logic_id, "statement-regen");
        let raw = handle.await.unwrap();
        assert!(raw.starts_with("POST /v1/workflows HTTP/1.1"), "got: {raw}");
        // no Authorization header on the platform path — the
        // identity rides the TLS handshake (absent in this plain-HTTP test
        // transport).
        assert!(
            !raw.contains("authorization:") && !raw.contains("Authorization:"),
            "no bearer header may be attached: {raw}"
        );
        assert!(raw.contains("\"logic_id\":\"statement-regen\""));
        assert!(raw.contains("\"correlation_id\":\"corr-1\""));
    }

    #[tokio::test]
    async fn list_workflows_builds_query_params() {
        install_provider();
        let (handle, addr) = serve("HTTP/1.1 200 OK", "[]").await;
        let c = FlowClient::plain(format!("http://{addr}"));
        let out = c
            .list_workflows(Some("ledger"), Some("failed"), Some("corr 1"))
            .await
            .unwrap();
        assert!(out.is_empty());
        let raw = handle.await.unwrap();
        assert!(
            raw.starts_with(
                "GET /v1/workflows?domain=ledger&state=failed&correlation_id=corr%201 HTTP/1.1"
            ),
            "got: {raw}"
        );
    }

    #[tokio::test]
    async fn retry_repair_cancel_resume_paths() {
        install_provider();
        for (path, action) in [
            ("/v1/workflows/w1/retry", "retry"),
            ("/v1/workflows/w1/repair", "repair"),
            ("/v1/workflows/w1/cancel", "cancel"),
            ("/v1/workflows/w1/resume", "resume"),
        ] {
            let (handle, addr) = serve("HTTP/1.1 200 OK", SAMPLE_WORKFLOW_JSON).await;
            let c = FlowClient::plain(format!("http://{addr}"));
            match action {
                "retry" => {
                    c.retry_workflow("w1").await.unwrap();
                }
                "repair" => {
                    c.repair_workflow("w1", 2, Some(&serde_json::json!({"x":1})))
                        .await
                        .unwrap();
                }
                "cancel" => {
                    c.cancel_workflow("w1").await.unwrap();
                }
                _ => {
                    c.resume_workflow("w1", Some(&serde_json::json!({"approve": true})))
                        .await
                        .unwrap();
                }
            }
            let raw = handle.await.unwrap();
            let expected = format!("POST {path} HTTP/1.1");
            assert!(raw.starts_with(&expected), "got: {raw}");
        }
    }

    #[tokio::test]
    async fn get_workflow_error_status_maps() {
        install_provider();
        let (handle, addr) = serve("HTTP/1.1 404 Not Found", "{}").await;
        let c = FlowClient::plain(format!("http://{addr}"));
        let err = c.get_workflow("missing").await.unwrap_err();
        assert!(
            matches!(err, FlowError::Http(ref m) if m.contains("404")),
            "{err:?}"
        );
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn malformed_json_maps_to_decode_error() {
        install_provider();
        let (handle, addr) = serve("HTTP/1.1 200 OK", "not-json").await;
        let c = FlowClient::plain(format!("http://{addr}"));
        let err = c.get_workflow("w1").await.unwrap_err();
        assert!(matches!(err, FlowError::Decode(_)), "{err:?}");
        handle.await.unwrap();
    }
}
