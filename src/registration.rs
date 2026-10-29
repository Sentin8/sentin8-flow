// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Sentin8:Flow
// SPDX-License-Identifier: Apache-2.0

//! Service self-registration data for Sentin8:Flow.
//!
//! Output via `--print-registration` flag. The CLI runs the binary with this
//! flag and consumes the JSON to register the service in the platform DB.
//! See the service design docs.

use sentin8::registration::{
    AdminScope, AdminUi, AdminUiPage, PgRole, PolicySetDecl, Registration, RelationDecl,
    SqlMigration,
};
use utoipa::OpenApi;

const PG_ROLES: &[PgRole] = &[PgRole {
    scope: "sentin8:flow:read,sentin8:flow:write",
    role: "sentin8_flow_service",
    bypass_rls: false,
    database: "sentin8",
    cross_schema_grants: &[],
}];

const ADMIN_UI_PAGES: &[AdminUiPage] = &[
    AdminUiPage {
        route: "overview",
        label: "Overview",
        ui_endpoint: "/v1/admin/ui/overview",
        data_endpoint: "/v1/admin/overview",
        icon: "0xe1b1",
    },
    AdminUiPage {
        route: "workflows",
        label: "Workflows",
        ui_endpoint: "/v1/admin/ui/workflows",
        data_endpoint: "/v1/admin/workflows",
        icon: "0xf0561",
    },
    AdminUiPage {
        route: "steps",
        label: "Steps",
        ui_endpoint: "/v1/admin/ui/steps",
        data_endpoint: "/v1/admin/steps",
        icon: "0xe37f",
    },
    AdminUiPage {
        route: "policy-sets",
        label: "Policy Sets",
        ui_endpoint: "/v1/admin/ui/policy-sets",
        data_endpoint: "/v1/admin/policy-sets",
        icon: "0xe8b8",
    },
    AdminUiPage {
        route: "policy-decisions",
        label: "Policy Decisions",
        ui_endpoint: "/v1/admin/ui/policy-decisions",
        data_endpoint: "/v1/admin/policy-decisions",
        icon: "0xf070",
    },
];

const SQL_1_INIT: &str = include_str!("../sql/init.sql");

/// Serialize the utoipa OpenAPI spec (used for the `all` and `openapi` sections).
fn openapi_spec() -> serde_json::Value {
    serde_json::to_value(crate::openapi::FlowOpenApi::openapi()).unwrap_or(serde_json::Value::Null)
}

/// Default declarative policy sets (Sentin8:Policy design /):
/// action-granular scope permits over the LIVE effective scope set. Seeded
/// `origin='platform'`, `mode='enforced'` ( - step 5 is
/// the only gate; shadow is the emergency one-UPDATE rollback lever only).
const POLICIES: &[PolicySetDecl] = &[
    PolicySetDecl {
        policy_set_name: "scope-mirror",
        description: "Action-granular scope permits for sentin8:flow over the LIVE effective scope set. Enforced - step 5 is the only gate.",
        policies: "permit(principal, action == Action::\"sentin8:flow:read\", resource) when { principal.scopes.contains(\"sentin8:flow:read\") };\npermit(principal, action == Action::\"sentin8:flow:read\", resource) when { principal.scopes.contains(\"sentin8:flow:write\") };\npermit(principal, action == Action::\"sentin8:flow:write\", resource) when { principal.scopes.contains(\"sentin8:flow:write\") };",
        schema: None,
        mode: "enforced",
        default_enabled: true,
    },
];

/// Default ReBAC relation tuples - none declared by this
/// service today; env admins author their own graph via the admin API.
const RELATIONS: &[RelationDecl] = &[];

/// Print the registration JSON to stdout.
///
/// With no `--section` argument, outputs the full registration object.
/// With `--section=<name>`, outputs only that section as a standalone JSON value.
/// M2M DCR workload grants (self-registration ): this
/// service's own `certless:<workload>` rows, upserted by `vertical enroll`.
/// The static auth seed is retired - this declaration is the ONLY
/// provisioning path ( "DCR is dynamic").
/// Cover BOTH the full-identity `-policy` spelling (policy_bus_client) and
/// the bare `{short}-policy` spelling (stat_emit); the gate's superset
/// check admits any requested subset.
const M2M_GRANTS: &[sentin8::registration::M2mGrant] = &[
    sentin8::registration::M2mGrant {
        workload: "flow-audit",
        scopes: &["sentin8:sys:audit:write"],
    },
    sentin8::registration::M2mGrant {
        workload: "sentin8:flow",
        scopes: &["sentin8:bus:produce", "sentin8:flow:write"],
    },
    // The proxy-back workload: carries the step-execution scope owning
    // services verify (`{owner}:steps:execute` vocabulary - each owner
    // declares its own gate; the map authorizes the subset actually used).
    sentin8::registration::M2mGrant {
        workload: "sentin8-flow-proxy-back",
        scopes: &["sentin8:flow:steps:execute"],
    },
    sentin8::registration::M2mGrant {
        workload: "sentin8:flow-policy",
        scopes: &["sentin8:bus:consume", "sentin8:stat:measure"],
    },
    sentin8::registration::M2mGrant {
        workload: "flow-policy",
        scopes: &["sentin8:bus:consume", "sentin8:stat:measure"],
    },
    // (design/sentin8-vertical-data-plane-contract.md §VDC.7): the database-free HostContextResolver’s resolve workload - cache miss calls Sys `GET /v1/resolve/hostname`.
    sentin8::registration::M2mGrant {
        workload: "sentin8-flow-tls",
        scopes: &["sentin8:sys:instances:read"],
    },
];

/// Handle print.
pub fn print(section: Option<String>) -> anyhow::Result<()> {
    let reg = Registration {
        service_type: "sentin8:flow",
        subdomain: "flow",
        health_path: "/v1/health",
        admin_scopes: &[
            // Granular vocabulary (flow design): handlers gate
            // reads on `sentin8:flow:read` and writes on `sentin8:flow:write`.
            AdminScope::Tuple("sentin8:flow:admin", "sentin8:flow:admin"),
            AdminScope::Tuple("sentin8:flow:admin", "sentin8:flow:read"),
            AdminScope::Tuple("sentin8:flow:admin", "sentin8:flow:write"),
            // Policy-admin surface (policy design ; per-service scope).
            AdminScope::Tuple("sentin8:flow:admin", "sentin8:flow:policy:read"),
            AdminScope::Tuple("sentin8:flow:admin", "sentin8:flow:policy:write"),
        ],
        error_codes: &[],
        consent_providers: &[],
        consent_flows: &[],
        admin_ui: Some(AdminUi {
            label: "Flow",
            icon: "0xe044",
            service_flag: "flow",
            pages: ADMIN_UI_PAGES,
        }),
        sql_migrations: vec![SqlMigration {
            version: 1,
            name: "init",
            sql: SQL_1_INIT,
        }],
        pg_roles: PG_ROLES,
        policies: POLICIES,
        relations: RELATIONS,
        event_triggers: &[],
        schedules: &[],
        audit_findings: None,
        service_actions: None,
        m2m_grants: Some(M2M_GRANTS),
        openapi_url: "/openapi.json",
        openapi: openapi_spec,
    };
    sentin8::registration::print_registration(section, &reg, openapi_spec)
}
