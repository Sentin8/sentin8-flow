// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Sentin8:Flow
// SPDX-License-Identifier: Apache-2.0

//! Func integration — call Func when a step's `resolve_func` needs WASM custom logic.

use sentin8_func_sdk::client::{CallbackConfig, ExecuteRequest, FuncClient};

/// Execute a WASM hook via Func, passing the step context as hook context.
#[allow(clippy::too_many_arguments)]
pub async fn execute_wasm(
    client: &FuncClient,
    module_id: &str,
    hook: &str,
    context: serde_json::Value,
    callback_base_url: Option<&str>,
    callback_auth_token: Option<&str>,
    callback_internal_path: Option<&str>,
    host_functions: Option<&std::collections::HashMap<String, String>>,
) -> Result<serde_json::Value, anyhow::Error> {
    let cb = if let (Some(url), Some(token), Some(path)) = (
        callback_base_url,
        callback_auth_token,
        callback_internal_path,
    ) {
        Some(CallbackConfig {
            base_url: url.to_string(),
            auth_token: token.to_string(),
            internal_path: path.to_string(),
            host_functions: host_functions.cloned().unwrap_or_default(),
        })
    } else {
        None
    };
    let req = ExecuteRequest {
        hook: hook.to_string(),
        context,
        callback_config: cb,
    };
    let resp = client.execute(module_id, &req).await?;
    Ok(resp)
}
