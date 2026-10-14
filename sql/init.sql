-- SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
-- SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
-- SPDX-FileCopyrightText: Copyright Contributors to the Sentin8:Flow
-- SPDX-License-Identifier: Apache-2.0

-- Sentin8:Flow — generic workflow execution engine.

-- flow.workflows: all workflow records across all services
CREATE SCHEMA IF NOT EXISTS flow;
CREATE TABLE IF NOT EXISTS flow.workflows (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    env_id  UUID NOT NULL,
    domain          TEXT NOT NULL,
    logic_id        TEXT NOT NULL,
    logic           JSONB NOT NULL DEFAULT '{}',
    state           TEXT NOT NULL DEFAULT 'pending' CHECK (state IN ('pending', 'processing', 'waiting', 'completed', 'failed', 'cancelled')),
    correlation_id  TEXT,
    input           JSONB NOT NULL DEFAULT '{}',
    output          JSONB,
    current_step    TEXT,
    step_results    JSONB NOT NULL DEFAULT '[]',
    error_message   TEXT,
    scheduled_at    TIMESTAMPTZ,
    callback_config JSONB NOT NULL DEFAULT '{}',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at    TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_flow_workflows_env ON flow.workflows (env_id);
CREATE INDEX IF NOT EXISTS idx_flow_workflows_domain ON flow.workflows (domain);
CREATE INDEX IF NOT EXISTS idx_flow_workflows_state ON flow.workflows (state);
CREATE INDEX IF NOT EXISTS idx_flow_workflows_correlation ON flow.workflows (correlation_id) WHERE correlation_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_flow_workflows_due ON flow.workflows (scheduled_at) WHERE state = 'waiting' AND scheduled_at IS NOT NULL;

-- Create idempotency (flow DESIGN §6, principles §6.7): a replayed create
-- with the same (env, logic, correlation) while the workflow is still
-- pending dedupes to the original row — at-least-once producers must not
-- spawn duplicate runs.
CREATE UNIQUE INDEX IF NOT EXISTS idx_flow_workflows_idem
    ON flow.workflows (env_id, logic_id, correlation_id)
    WHERE state = 'pending' AND correlation_id IS NOT NULL;

-- Enable RLS
ALTER TABLE flow.workflows ENABLE ROW LEVEL SECURITY;
ALTER TABLE flow.workflows FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS flow_workflows_select ON flow.workflows;
CREATE POLICY flow_workflows_select ON flow.workflows FOR SELECT USING (sentin8_context_initialized() AND (sentin8_bypass_rls() OR (env_id = sentin8_current_env_id())));
DROP POLICY IF EXISTS flow_workflows_insert ON flow.workflows;
CREATE POLICY flow_workflows_insert ON flow.workflows FOR INSERT WITH CHECK (sentin8_context_initialized() AND (sentin8_bypass_rls() OR (env_id = sentin8_current_env_id())));
DROP POLICY IF EXISTS flow_workflows_update ON flow.workflows;
CREATE POLICY flow_workflows_update ON flow.workflows FOR UPDATE USING (sentin8_context_initialized() AND (sentin8_bypass_rls() OR (env_id = sentin8_current_env_id())));
DROP POLICY IF EXISTS flow_workflows_delete ON flow.workflows;
CREATE POLICY flow_workflows_delete ON flow.workflows FOR DELETE USING (sentin8_context_initialized() AND (sentin8_bypass_rls() OR (env_id = sentin8_current_env_id())));

-- ── Grants ──

-- sentin8_flow_service: DML on all flow tables.
GRANT SELECT, INSERT, UPDATE, DELETE ON flow.workflows TO sentin8_flow_service;


-- ── Local outbox (DBX, architecture-principles §6.7) ─────────────────────
-- Producer path: flow.outbox rows commit in the SAME transaction as the
-- domain change; the shell-spawned relay (shell boot step 8b, LISTEN
-- flow_outbox_changed + 5s safety poll) pushes them to Bus
-- POST /v1/produce (M2M, scope sentin8:bus:produce) and marks them relayed.
-- The same DBX producer path every shell service uses.
-- Generated from sentin8-outbox::outbox_table_sql("flow").
CREATE TABLE IF NOT EXISTS flow.outbox (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    env_id          UUID NOT NULL,
    event_type      TEXT NOT NULL,
    stream_family   TEXT NOT NULL,
    payload         JSONB NOT NULL,
    publisher_id_token  TEXT,
    publisher_subject   TEXT,
    relayed         BOOLEAN NOT NULL DEFAULT false,
    relayed_at      TIMESTAMPTZ,
    relay_attempts  INT NOT NULL DEFAULT 0,
    next_retry_at   TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_flow_outbox_unrelayed
    ON flow.outbox (created_at) WHERE relayed = false;
ALTER TABLE flow.outbox ENABLE ROW LEVEL SECURITY;
ALTER TABLE flow.outbox FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS outbox_select ON flow.outbox;
CREATE POLICY outbox_select ON flow.outbox FOR SELECT
    USING (sentin8_context_initialized() AND env_id = sentin8_current_env_id());
DROP POLICY IF EXISTS outbox_insert ON flow.outbox;
CREATE POLICY outbox_insert ON flow.outbox FOR INSERT
    WITH CHECK (sentin8_context_initialized() AND env_id = sentin8_current_env_id());
DROP POLICY IF EXISTS outbox_update ON flow.outbox;
CREATE POLICY outbox_update ON flow.outbox FOR UPDATE
    USING (sentin8_context_initialized() AND env_id = sentin8_current_env_id());
CREATE OR REPLACE FUNCTION flow.flow_notify_outbox() RETURNS TRIGGER
LANGUAGE plpgsql AS $fn$
BEGIN
    PERFORM pg_notify('flow_outbox_changed', '');
    RETURN NEW;
END;
$fn$;
DROP TRIGGER IF EXISTS outbox_notify_trigger ON flow.outbox;
CREATE TRIGGER outbox_notify_trigger
    AFTER INSERT ON flow.outbox
    FOR EACH ROW
    EXECUTE FUNCTION flow.flow_notify_outbox();

-- ── Local audit journal (DBX §6.8) ────────────────────────────────────────
-- Spill journal for the canonical audit push: rows written same-tx as the
-- domain change, pushed to Sys POST /v1/audit (sentin8:sys:audit:write), then
-- marked pushed. A Sys outage never drops audit. Generated from
-- sentin8-audit::audit_journal_sql("flow").
CREATE TABLE IF NOT EXISTS flow.audit_journal (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    env_id          UUID NOT NULL,
    action          TEXT NOT NULL,
    target_resource_type TEXT,
    target_resource_id    TEXT,
    actor_user_id   TEXT,
    actor_subject_type TEXT,
    severity        TEXT NOT NULL DEFAULT 'info',
    correlation_id  TEXT,
    before_state    JSONB,
    after_state     JSONB,
    details         JSONB,
    pushed          BOOLEAN NOT NULL DEFAULT false,
    pushed_at       TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_flow_audit_journal_unpushed
    ON flow.audit_journal (created_at) WHERE pushed = false;
ALTER TABLE flow.audit_journal ENABLE ROW LEVEL SECURITY;
ALTER TABLE flow.audit_journal FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS audit_journal_dml ON flow.audit_journal;
CREATE POLICY audit_journal_dml ON flow.audit_journal
    USING (sentin8_context_initialized() AND env_id = sentin8_current_env_id())
    WITH CHECK (sentin8_context_initialized() AND env_id = sentin8_current_env_id());


-- ── Schema grants (service-owned — the 2026-10-03 ownership pass) ─────
-- This service's own schema: USAGE for its role (platform.sql no longer
-- seeds per-service USAGE; the service declares it here).
GRANT USAGE ON SCHEMA flow TO sentin8_flow_service;
GRANT SELECT, INSERT, UPDATE, DELETE ON flow.outbox TO sentin8_flow_service;
GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA flow TO sentin8_flow_service;
GRANT SELECT, INSERT, UPDATE, DELETE ON flow.audit_journal TO sentin8_flow_service;
