CREATE EXTENSION IF NOT EXISTS pg_trgm;
CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE OR REPLACE FUNCTION prevent_update_trigger()
    RETURNS TRIGGER AS
$$
BEGIN
    raise exception
        'trigger %: updating is prohibited for %.%',
        tg_name, tg_table_schema, tg_table_name
        using errcode = 'restrict_violation';
END;
$$ LANGUAGE plpgsql;

CREATE TABLE users
(
    user_id       UUID PRIMARY KEY,
    username      TEXT        NOT NULL UNIQUE,
    password_hash TEXT,
    display_name  TEXT,
    email         TEXT,
    adm_status    SMALLINT    NOT NULL DEFAULT 1,
    allow_sso     BOOLEAN     NOT NULL DEFAULT false,
    oidc_subject  TEXT UNIQUE,
    last_updated  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_users_1 ON users (adm_status);
CREATE INDEX idx_users_2 ON users USING GIN (username gin_trgm_ops);

CREATE TABLE refresh_tokens
(
    token_hash BYTEA PRIMARY KEY
        CONSTRAINT refresh_tokens_token_hash_is_sha256 CHECK (octet_length(token_hash) = 32),
    user_id    UUID        NOT NULL REFERENCES users (user_id) ON DELETE CASCADE,
    expires_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX refresh_tokens_user_id_idx ON refresh_tokens (user_id);
CREATE INDEX refresh_tokens_expires_at_idx ON refresh_tokens (expires_at);

CREATE TABLE cluster_nodes
(
    node_id        UUID PRIMARY KEY,
    name           TEXT        NOT NULL,
    op_status      SMALLINT    NOT NULL,
    is_leader      BOOLEAN     NOT NULL DEFAULT false,
    last_heartbeat TIMESTAMPTZ NOT NULL,
    metadata       JSONB       NOT NULL
);
CREATE UNIQUE INDEX idx_cluster_nodes_leader ON cluster_nodes (is_leader) WHERE is_leader = true;

CREATE TABLE cluster_standby_nodes
(
    node_id        UUID,
    name           TEXT,
    last_heartbeat TIMESTAMPTZ NOT NULL,
    metadata       JSONB       NOT NULL,
    PRIMARY KEY (node_id, name)
);

CREATE TABLE cluster_logs
(
    ts          TIMESTAMPTZ NOT NULL DEFAULT now(),
    node_id     UUID        NOT NULL REFERENCES cluster_nodes (node_id),
    resource    TEXT        NOT NULL,
    resource_id UUID        NOT NULL,
    op          SMALLINT    NOT NULL,
    seq         BIGINT GENERATED ALWAYS AS IDENTITY
);
CREATE INDEX idx_cluster_logs_1 ON cluster_logs (node_id, resource, resource_id);

CREATE TABLE hosts
(
    host_id             UUID PRIMARY KEY,
    name                TEXT        NOT NULL,
    adm_status          SMALLINT    NOT NULL,
    labels              JSONB,
    last_updated        TIMESTAMPTZ NOT NULL,
    monitored_by        UUID REFERENCES cluster_nodes (node_id),
    sticky_monitored_by BOOLEAN     NOT NULL DEFAULT false,
    timezone            TEXT
);
CREATE INDEX idx_hosts_1 ON hosts USING GIN (name gin_trgm_ops);
CREATE INDEX idx_hosts_2 ON hosts USING GIN (labels);
CREATE INDEX idx_hosts_3 ON hosts (adm_status);
CREATE INDEX idx_hosts_4 ON hosts (monitored_by);

CREATE OR REPLACE FUNCTION log_host_cache_operation(
    p_node_id UUID,
    p_host_id UUID,
    p_op INTEGER,
    p_include_children BOOLEAN DEFAULT false
)
    RETURNS VOID AS
$$
BEGIN
    INSERT INTO cluster_logs (node_id, resource, resource_id, op)
    VALUES (p_node_id, 'host', p_host_id, p_op);

    IF p_include_children THEN
        INSERT INTO cluster_logs (node_id, resource, resource_id, op)
        SELECT p_node_id, 'host_endpoint', endpoint_id, p_op
        FROM host_endpoints
        WHERE host_id = p_host_id
          AND adm_status = 1;

        INSERT INTO cluster_logs (node_id, resource, resource_id, op)
        SELECT p_node_id, 'item', item_id, p_op
        FROM items i
                 JOIN host_endpoints e ON e.endpoint_id = i.endpoint_id AND
                                          e.host_id = p_host_id AND
                                          e.adm_status = 1
        WHERE i.adm_status = 1;

        INSERT INTO cluster_logs (node_id, resource, resource_id, op)
        SELECT p_node_id, 'item', item_id, p_op
        FROM items i
                 JOIN host_templates ht ON ht.host_template_id = i.host_template_id AND
                                           ht.host_id = p_host_id
        WHERE endpoint_id IS NULL
          AND adm_status = 1;
    END IF;
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION log_listener_host_cache_operation(
    p_host_id UUID,
    p_op INTEGER
)
    RETURNS VOID AS
$$
BEGIN
    INSERT INTO cluster_logs (node_id, resource, resource_id, op)
    SELECT cn.node_id, 'listener_item', i.item_id, p_op
    FROM cluster_nodes cn,
         items i
             JOIN host_templates ht on ht.host_template_id = i.host_template_id AND
                                       ht.host_id = p_host_id
    WHERE i.adm_status = 1
      AND i.type = 'listener'
      AND i.listener_id IS NOT NULL
      AND (i.endpoint_id IS NULL
        OR EXISTS (SELECT 1
                   FROM host_endpoints he
                   WHERE he.endpoint_id = i.endpoint_id
                     AND he.adm_status = 1));

    INSERT INTO cluster_logs (node_id, resource, resource_id, op)
    SELECT cn.node_id, 'listener_endpoint', he.endpoint_id, p_op
    FROM cluster_nodes cn,
         host_endpoints he
    WHERE he.host_id = p_host_id
      AND he.adm_status = 1
      AND EXISTS (SELECT 1
                  FROM items i
                  WHERE i.endpoint_id = he.endpoint_id
                    AND i.adm_status = 1
                    AND i.type = 'listener'
                    AND i.listener_id IS NOT NULL);
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION log_host_cluster_changes()
    RETURNS TRIGGER AS
$$
BEGIN
    IF TG_OP = 'DELETE' AND OLD.monitored_by IS NOT NULL AND OLD.adm_status = 1 THEN
        PERFORM log_host_cache_operation(OLD.monitored_by, OLD.host_id, 2, false);
        RETURN OLD;
    END IF;

    IF TG_OP = 'INSERT' AND NEW.monitored_by IS NOT NULL AND NEW.adm_status = 1 THEN
        PERFORM log_host_cache_operation(NEW.monitored_by, NEW.host_id, 1, false);
        RETURN NEW;
    END IF;

    IF TG_OP = 'UPDATE' THEN
        -- The listener caches take only adm_status and monitored_by from a host
        -- (item entries carry monitored_by). Other host changes skip these logs,
        -- which go to every node.
        IF OLD.adm_status IS DISTINCT FROM NEW.adm_status THEN
            IF NEW.adm_status = 1 THEN
                PERFORM log_listener_host_cache_operation(NEW.host_id, 1);
            ELSIF OLD.adm_status = 1 THEN
                PERFORM log_listener_host_cache_operation(NEW.host_id, 2);
            END IF;
        ELSIF NEW.adm_status = 1
            AND OLD.monitored_by IS DISTINCT FROM NEW.monitored_by THEN
            PERFORM log_listener_host_cache_operation(NEW.host_id, 1);
        END IF;

        IF OLD.monitored_by IS DISTINCT FROM NEW.monitored_by AND NEW.adm_status = 1 THEN
            IF OLD.monitored_by IS NOT NULL THEN
                PERFORM log_host_cache_operation(OLD.monitored_by, OLD.host_id, 2, true);
            END IF;

            IF NEW.monitored_by IS NOT NULL THEN
                PERFORM log_host_cache_operation(NEW.monitored_by, NEW.host_id, 1, true);
            END IF;

            RETURN NEW;
        END IF;

        IF OLD.adm_status IS DISTINCT FROM NEW.adm_status AND NEW.monitored_by IS NOT NULL THEN
            IF NEW.adm_status = 1 THEN
                PERFORM log_host_cache_operation(NEW.monitored_by, NEW.host_id, 1, true);
            ELSE
                PERFORM log_host_cache_operation(NEW.monitored_by, NEW.host_id, 2, true);
            END IF;

            RETURN NEW;
        END IF;

        IF NEW.monitored_by IS NOT NULL AND NEW.adm_status = 1 THEN
            PERFORM log_host_cache_operation(NEW.monitored_by, NEW.host_id, 1, false);
            RETURN NEW;
        END IF;
    END IF;

    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_log_host_cluster_changes
    AFTER INSERT OR UPDATE OR DELETE
    ON hosts
    FOR EACH ROW
EXECUTE FUNCTION log_host_cluster_changes();

CREATE TABLE templates
(
    template_id UUID PRIMARY KEY,
    name        TEXT NOT NULL,
    labels      JSONB
);
CREATE INDEX idx_templates_1 ON templates USING GIN (name gin_trgm_ops);
CREATE INDEX idx_templates_2 ON templates USING GIN (labels);

CREATE TABLE template_links
(
    template_link_id    UUID PRIMARY KEY,
    template_id         UUID NOT NULL REFERENCES templates (template_id),
    link_to_template_id UUID NOT NULL REFERENCES templates (template_id)
);
CREATE INDEX idx_template_links_1 ON template_links (template_id);
CREATE INDEX idx_template_links_2 ON template_links (link_to_template_id);
CREATE UNIQUE INDEX idx_template_links_3 ON template_links (template_id, link_to_template_id);

CREATE TABLE host_templates
(
    host_template_id UUID PRIMARY KEY,
    host_id          UUID REFERENCES hosts (host_id),
    template_id      UUID REFERENCES templates (template_id),
    CONSTRAINT valid_entry CHECK ( num_nonnulls(host_id, template_id) > 0 )
);
CREATE INDEX idx_host_templates_1 ON host_templates (host_id);
CREATE INDEX idx_host_templates_2 ON host_templates (template_id);
CREATE UNIQUE INDEX idx_host_templates_3 ON host_templates (host_id) WHERE template_id IS NULL;
CREATE UNIQUE INDEX idx_host_templates_4 ON host_templates (template_id) WHERE host_id IS NULL;
CREATE UNIQUE INDEX idx_host_templates_5 ON host_templates (host_id, template_id) WHERE host_id IS NOT NULL AND template_id IS NOT NULL;

CREATE TABLE listeners
(
    listener_id                UUID PRIMARY KEY,
    name                       TEXT        NOT NULL,
    type                       TEXT        NOT NULL,
    adm_status                 SMALLINT    NOT NULL,
    labels                     JSONB,
    last_updated               TIMESTAMPTZ NOT NULL,
    worker_count               INTEGER     NOT NULL DEFAULT 1,
    config                     JSONB       NOT NULL,
    disable_cluster_forwarding BOOLEAN     NOT NULL DEFAULT FALSE,
    batch_size                 INTEGER     NOT NULL DEFAULT 1000,
    batch_timeout_ms           INTEGER     NOT NULL DEFAULT 1000,
    CONSTRAINT valid_worker_count CHECK ( worker_count >= 1 ),
    CONSTRAINT valid_batch_size CHECK ( batch_size >= 1 ),
    CONSTRAINT valid_batch_timeout_ms CHECK ( batch_timeout_ms >= 1 )
);
CREATE INDEX idx_listeners_1 ON listeners USING GIN (name gin_trgm_ops);
CREATE INDEX idx_listeners_2 ON listeners USING GIN (labels);
CREATE INDEX idx_listeners_3 ON listeners (adm_status);
CREATE INDEX idx_listeners_4 ON listeners (last_updated);

CREATE OR REPLACE FUNCTION log_listener_cluster_changes()
    RETURNS TRIGGER AS
$$
BEGIN
    IF TG_OP = 'DELETE' AND OLD.adm_status = 1 THEN
        INSERT INTO cluster_logs (node_id, resource, resource_id, op)
        SELECT node_id, 'listener', OLD.listener_id, 2
        FROM cluster_nodes;
    ELSIF TG_OP = 'UPDATE' AND OLD.adm_status = 1 AND NEW.adm_status <> 1 THEN
        INSERT INTO cluster_logs (node_id, resource, resource_id, op)
        SELECT node_id, 'listener', NEW.listener_id, 2
        FROM cluster_nodes;
    ELSIF TG_OP IN ('INSERT', 'UPDATE') AND NEW.adm_status = 1 THEN
        INSERT INTO cluster_logs (node_id, resource, resource_id, op)
        SELECT node_id, 'listener', NEW.listener_id, 1
        FROM cluster_nodes;
    END IF;

    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_log_listener_cluster_changes
    AFTER INSERT OR UPDATE OR DELETE
    ON listeners
    FOR EACH ROW
EXECUTE FUNCTION log_listener_cluster_changes();

CREATE OR REPLACE FUNCTION log_cluster_node_cluster_changes()
    RETURNS TRIGGER AS
$$
BEGIN
    IF TG_OP = 'DELETE' THEN
        INSERT INTO cluster_logs (node_id, resource, resource_id, op)
        SELECT node_id, 'cluster_node', OLD.node_id, 2
        FROM cluster_nodes
        WHERE node_id <> OLD.node_id;
        RETURN OLD;
    ELSIF TG_OP = 'INSERT' THEN
        INSERT INTO cluster_logs (node_id, resource, resource_id, op)
        SELECT node_id, 'cluster_node', NEW.node_id, 1
        FROM cluster_nodes
        WHERE node_id <> NEW.node_id;
    ELSIF TG_OP = 'UPDATE'
        AND (OLD.metadata IS DISTINCT FROM NEW.metadata
            OR OLD.op_status IS DISTINCT FROM NEW.op_status) THEN
        INSERT INTO cluster_logs (node_id, resource, resource_id, op)
        SELECT node_id, 'cluster_node', NEW.node_id, 1
        FROM cluster_nodes
        WHERE node_id <> NEW.node_id;
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_log_cluster_node_cluster_changes
    AFTER INSERT OR UPDATE OR DELETE
    ON cluster_nodes
    FOR EACH ROW
EXECUTE FUNCTION log_cluster_node_cluster_changes();

CREATE TABLE host_endpoints
(
    endpoint_id  UUID PRIMARY KEY,
    host_id      UUID        NOT NULL REFERENCES hosts (host_id),
    name         TEXT        NOT NULL,
    type         TEXT        NOT NULL,
    adm_status   SMALLINT    NOT NULL,
    last_updated TIMESTAMPTZ NOT NULL,
    config       JSONB       NOT NULL
);
CREATE INDEX idx_host_endpoints_1 ON host_endpoints (host_id);
CREATE INDEX idx_host_endpoints_2 ON host_endpoints (last_updated);
CREATE INDEX idx_host_endpoints_3 ON host_endpoints (adm_status);

CREATE OR REPLACE FUNCTION log_endpoint_cache_operation(
    p_node_id UUID,
    p_endpoint_id UUID,
    p_op INTEGER,
    p_include_children BOOLEAN DEFAULT false
)
    RETURNS VOID AS
$$
BEGIN
    INSERT INTO cluster_logs (node_id, resource, resource_id, op)
    VALUES (p_node_id, 'host_endpoint', p_endpoint_id, p_op);

    IF p_include_children THEN
        INSERT INTO cluster_logs (node_id, resource, resource_id, op)
        SELECT p_node_id, 'item', item_id, p_op
        FROM items
        WHERE adm_status = 1
          AND endpoint_id = p_endpoint_id;
    END IF;
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION log_listener_endpoint_1_cache_operation(
    p_endpoint_id UUID,
    p_op INTEGER
)
    RETURNS VOID AS
$$
BEGIN
    INSERT INTO cluster_logs (node_id, resource, resource_id, op)
    SELECT cn.node_id, 'listener_endpoint', p_endpoint_id, p_op
    FROM cluster_nodes cn
    WHERE EXISTS(SELECT 1
                 FROM items i
                 WHERE i.adm_status = 1
                   AND i.endpoint_id = p_endpoint_id
                   AND i.type = 'listener'
                   AND i.listener_id IS NOT NULL);

    INSERT INTO cluster_logs (node_id, resource, resource_id, op)
    SELECT cn.node_id, 'listener_item', i.item_id, p_op
    FROM cluster_nodes cn,
         items i
    WHERE i.adm_status = 1
      AND i.endpoint_id = p_endpoint_id
      AND i.type = 'listener'
      AND i.listener_id IS NOT NULL;
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION log_endpoint_cluster_changes()
    RETURNS TRIGGER AS
$$
DECLARE
    host_rec hosts%ROWTYPE;
BEGIN
    IF TG_OP = 'INSERT' OR TG_OP = 'UPDATE' THEN
        SELECT * INTO host_rec FROM hosts WHERE host_id = NEW.host_id AND adm_status = 1;
    ELSE
        SELECT * INTO host_rec FROM hosts WHERE host_id = OLD.host_id AND adm_status = 1;
    END IF;
    IF host_rec IS NULL THEN
        RETURN NULL;
    END IF;

    IF TG_OP = 'DELETE' AND host_rec.monitored_by IS NOT NULL AND OLD.adm_status = 1 THEN
        PERFORM log_endpoint_cache_operation(host_rec.monitored_by, OLD.endpoint_id, 2, false);
        RETURN OLD;
    END IF;

    IF TG_OP = 'INSERT' AND host_rec.monitored_by IS NOT NULL AND NEW.adm_status = 1 THEN
        PERFORM log_endpoint_cache_operation(host_rec.monitored_by, NEW.endpoint_id, 1, false);
        RETURN NEW;
    END IF;

    IF TG_OP = 'UPDATE' THEN
        IF OLD.adm_status IS DISTINCT FROM NEW.adm_status THEN
            IF NEW.adm_status = 1 THEN
                PERFORM log_listener_endpoint_1_cache_operation(NEW.endpoint_id, 1);
            ELSIF OLD.adm_status = 1 THEN
                PERFORM log_listener_endpoint_1_cache_operation(NEW.endpoint_id, 2);
            END IF;
        ELSIF NEW.adm_status = 1
            AND (OLD.config -> 'address') IS DISTINCT FROM (NEW.config -> 'address') THEN
            PERFORM log_listener_endpoint_1_cache_operation(NEW.endpoint_id, 1);
        END IF;

        IF OLD.adm_status IS DISTINCT FROM NEW.adm_status AND host_rec.monitored_by IS NOT NULL THEN
            IF NEW.adm_status = 1 THEN
                PERFORM log_endpoint_cache_operation(host_rec.monitored_by, NEW.endpoint_id, 1, true);
            ELSE
                PERFORM log_endpoint_cache_operation(host_rec.monitored_by, NEW.endpoint_id, 2, true);
            END IF;
            RETURN NEW;
        END IF;

        IF host_rec.monitored_by IS NOT NULL AND NEW.adm_status = 1 THEN
            PERFORM log_endpoint_cache_operation(host_rec.monitored_by, NEW.endpoint_id, 1, false);
            RETURN NEW;
        END IF;
    END IF;

    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_log_endpoint_cluster_changes
    AFTER INSERT OR UPDATE OR DELETE
    ON host_endpoints
    FOR EACH ROW
EXECUTE FUNCTION log_endpoint_cluster_changes();

CREATE TABLE processors
(
    processor_id UUID PRIMARY KEY,
    name         TEXT        NOT NULL,
    type         TEXT        NOT NULL,
    adm_status   SMALLINT    NOT NULL,
    last_updated TIMESTAMPTZ NOT NULL,
    config       JSONB       NOT NULL
);
CREATE INDEX idx_processors_1 ON processors (last_updated);
CREATE INDEX idx_processors_2 ON processors (adm_status);

CREATE TABLE processor_inputs
(
    processor_id UUID NOT NULL REFERENCES processors (processor_id),
    input_id     UUID NOT NULL REFERENCES processors (processor_id),
    PRIMARY KEY (processor_id, input_id)
);
CREATE INDEX idx_processor_inputs_1 ON processor_inputs (processor_id);
CREATE INDEX idx_processor_inputs_2 ON processor_inputs (input_id);

CREATE TRIGGER processor_inputs_prevent_update
    BEFORE UPDATE
    on processor_inputs
    FOR EACH ROW
EXECUTE FUNCTION prevent_update_trigger();

CREATE OR REPLACE FUNCTION log_processor_cluster_changes()
    RETURNS TRIGGER AS
$$
DECLARE
    p_processor_id UUID;
BEGIN
    IF TG_TABLE_NAME = 'processors' THEN
        IF TG_OP = 'DELETE' THEN
            INSERT INTO cluster_logs (node_id, resource, resource_id, op)
            SELECT node_id, 'processor', OLD.processor_id, 2
            FROM cluster_nodes;
        ELSIF TG_OP IN ('INSERT', 'UPDATE') THEN
            INSERT INTO cluster_logs (node_id, resource, resource_id, op)
            SELECT node_id, 'processor', NEW.processor_id, 1
            FROM cluster_nodes;
        END IF;
    END IF;
    IF TG_TABLE_NAME = 'processor_inputs' THEN
        IF TG_OP = 'DELETE' THEN
            p_processor_id = OLD.processor_id;
        ELSE
            p_processor_id = NEW.processor_id;
        END IF;
        INSERT INTO cluster_logs (node_id, resource, resource_id, op)
        SELECT node_id, 'processor', p_processor_id, 1
        FROM cluster_nodes;
    END IF;

    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_log_processor_cluster_changes
    AFTER INSERT OR UPDATE OR DELETE
    ON processors
    FOR EACH ROW
EXECUTE FUNCTION log_processor_cluster_changes();

CREATE TRIGGER trg_log_processor_input_cluster_changes
    AFTER INSERT OR UPDATE OR DELETE
    ON processor_inputs
    FOR EACH ROW
EXECUTE FUNCTION log_processor_cluster_changes();

CREATE TABLE items
(
    item_id          UUID PRIMARY KEY,
    host_template_id UUID        NOT NULL REFERENCES host_templates (host_template_id),
    template_item_id UUID REFERENCES items (item_id),
    -- listener_id and endpoint_id are null for a template item; only the API
    -- enforces this
    listener_id      UUID REFERENCES listeners (listener_id),
    endpoint_id      UUID REFERENCES host_endpoints (endpoint_id),
    type             TEXT        NOT NULL,
    flags            INTEGER     NOT NULL DEFAULT 0,
    adm_status       SMALLINT    NOT NULL,
    name             TEXT,
    schedule         TEXT,
    timeout_ms       INTEGER,
    last_updated     TIMESTAMPTZ NOT NULL,
    config           JSONB,
    labels           JSONB,
    CONSTRAINT valid_nulls_1 CHECK ( num_nonnulls(name, template_item_id) > 0 ),
    CONSTRAINT valid_nulls_2 CHECK ( num_nonnulls(schedule, template_item_id) > 0 ),
    CONSTRAINT valid_nulls_3 CHECK ( num_nonnulls(timeout_ms, template_item_id) > 0 ),
    CONSTRAINT valid_nulls_4 CHECK ( num_nonnulls(config, template_item_id) > 0 ),
    CONSTRAINT valid_timeout CHECK ( timeout_ms IS NULL OR timeout_ms >= 0 )
);
CREATE INDEX idx_items_1 ON items (host_template_id);
CREATE INDEX idx_items_2 ON items USING GIN (labels);
CREATE INDEX idx_items_3 ON items (last_updated);
CREATE INDEX idx_items_4 ON items USING GIN (name gin_trgm_ops);
CREATE INDEX idx_items_5 ON items (adm_status);
CREATE INDEX idx_items_6 ON items (template_item_id);
CREATE INDEX idx_items_7 ON items (listener_id);
CREATE INDEX idx_items_8 ON items (endpoint_id);

CREATE TABLE item_processors
(
    item_id      UUID NOT NULL REFERENCES items (item_id),
    processor_id UUID NOT NULL REFERENCES processors (processor_id),
    PRIMARY KEY (item_id, processor_id)
);
CREATE INDEX idx_item_processors_1 ON item_processors (item_id);
CREATE INDEX idx_item_processors_2 ON item_processors (processor_id);

CREATE TRIGGER item_processors_prevent_update
    BEFORE UPDATE
    on item_processors
    FOR EACH ROW
EXECUTE FUNCTION prevent_update_trigger();

CREATE OR REPLACE FUNCTION log_item_cache_operation(
    p_node_id UUID,
    p_item_id UUID,
    p_op INTEGER
)
    RETURNS VOID AS
$$
BEGIN
    INSERT INTO cluster_logs (node_id, resource, resource_id, op)
    VALUES (p_node_id, 'item', p_item_id, p_op);
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION log_listener_item_cache_operation(
    p_item_id UUID,
    p_op INTEGER
)
    RETURNS VOID AS
$$
BEGIN
    INSERT INTO cluster_logs (node_id, resource, resource_id, op)
    SELECT node_id, 'listener_item', p_item_id, p_op
    FROM cluster_nodes;
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION log_listener_endpoint_2_cache_operation(
    p_endpoint_id UUID,
    p_op INTEGER
)
    RETURNS VOID AS
$$
BEGIN
    INSERT INTO cluster_logs (node_id, resource, resource_id, op)
    SELECT cn.node_id, 'listener_endpoint', p_endpoint_id, p_op
    FROM cluster_nodes cn;
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION item_active_host(
    p_endpoint_id UUID,
    p_host_template_id UUID,
    OUT o_host_id UUID,
    OUT o_monitored_by UUID
) AS
$$
BEGIN
    IF p_endpoint_id IS NOT NULL THEN
        SELECT h.host_id, h.monitored_by
        INTO o_host_id, o_monitored_by
        FROM host_endpoints he
                 JOIN hosts h ON h.host_id = he.host_id AND
                                 h.adm_status = 1
        WHERE he.endpoint_id = p_endpoint_id
          AND he.adm_status = 1;
    ELSE
        SELECT h.host_id, h.monitored_by
        INTO o_host_id, o_monitored_by
        FROM host_templates ht
                 JOIN hosts h ON h.host_id = ht.host_id AND
                                 h.adm_status = 1
        WHERE ht.host_template_id = p_host_template_id;
    END IF;
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION log_item_cluster_changes()
    RETURNS TRIGGER AS
$$
DECLARE
    old_host_id   UUID;
    old_node      UUID;
    old_listening BOOLEAN := false;
    new_host_id   UUID;
    new_node      UUID;
    new_listening BOOLEAN := false;
    r2            RECORD;
BEGIN
    IF TG_OP <> 'INSERT' THEN
        SELECT o_host_id, o_monitored_by
        INTO old_host_id, old_node
        FROM item_active_host(OLD.endpoint_id, OLD.host_template_id);
        IF OLD.adm_status <> 1 THEN
            old_node := NULL;
        END IF;
        old_listening := old_host_id IS NOT NULL
            AND OLD.adm_status = 1
            AND OLD.type = 'listener'
            AND OLD.listener_id IS NOT NULL;
    END IF;
    IF TG_OP <> 'DELETE' THEN
        SELECT o_host_id, o_monitored_by
        INTO new_host_id, new_node
        FROM item_active_host(NEW.endpoint_id, NEW.host_template_id);
        IF NEW.adm_status <> 1 THEN
            new_node := NULL;
        END IF;
        new_listening := new_host_id IS NOT NULL
            AND NEW.adm_status = 1
            AND NEW.type = 'listener'
            AND NEW.listener_id IS NOT NULL;
    END IF;

    IF old_node IS NOT NULL AND old_node IS DISTINCT FROM new_node THEN
        PERFORM log_item_cache_operation(old_node, OLD.item_id, 2);
    END IF;
    IF new_node IS NOT NULL THEN
        PERFORM log_item_cache_operation(new_node, NEW.item_id, 1);
    END IF;

    IF old_listening AND NOT new_listening THEN
        PERFORM log_listener_item_cache_operation(OLD.item_id, 2);
    END IF;
    IF old_listening
        AND OLD.endpoint_id IS NOT NULL
        AND (NOT new_listening OR NEW.endpoint_id IS DISTINCT FROM OLD.endpoint_id)
        AND NOT EXISTS (SELECT 1
                        FROM items i
                        WHERE i.endpoint_id = OLD.endpoint_id
                          AND i.type = 'listener'
                          AND i.listener_id IS NOT NULL
                          AND i.adm_status = 1) THEN
        PERFORM log_listener_endpoint_2_cache_operation(OLD.endpoint_id, 2);
    END IF;
    IF new_listening THEN
        IF NEW.endpoint_id IS NOT NULL AND
           NOT EXISTS (SELECT 1
                       FROM items i
                       WHERE i.endpoint_id = NEW.endpoint_id
                         AND i.item_id <> NEW.item_id
                         AND i.type = 'listener'
                         AND i.listener_id IS NOT NULL
                         AND i.adm_status = 1) THEN
            PERFORM log_listener_endpoint_2_cache_operation(NEW.endpoint_id, 1);
        END IF;
        PERFORM log_listener_item_cache_operation(NEW.item_id, 1);
    END IF;

    IF TG_OP = 'UPDATE' AND EXISTS (SELECT 1
                                    FROM host_templates ht
                                    WHERE ht.host_template_id = NEW.host_template_id
                                      AND ht.host_id IS NULL) THEN
        FOR r2 IN
            WITH RECURSIVE item_chain AS (SELECT i.item_id,
                                                 i.type,
                                                 i.listener_id,
                                                 ht.host_id,
                                                 h.monitored_by,
                                                 h.adm_status  as host_adm_status,
                                                 i.endpoint_id,
                                                 he.adm_status as endpoint_adm_status
                                          FROM items i
                                                   JOIN host_templates ht on ht.host_template_id = i.host_template_id
                                                   LEFT JOIN hosts h on h.host_id = ht.host_id
                                                   LEFT JOIN host_endpoints he on he.endpoint_id = i.endpoint_id
                                          WHERE i.item_id = NEW.item_id
                                          UNION ALL
                                          SELECT p.item_id,
                                                 p.type,
                                                 p.listener_id,
                                                 ht.host_id,
                                                 h.monitored_by,
                                                 h.adm_status  as host_adm_status,
                                                 p.endpoint_id,
                                                 he.adm_status as endpoint_adm_status
                                          FROM item_chain c
                                                   JOIN items p on p.template_item_id = c.item_id
                                                   JOIN host_templates ht on ht.host_template_id = p.host_template_id
                                                   LEFT JOIN hosts h on h.host_id = ht.host_id
                                                   LEFT JOIN host_endpoints he on he.endpoint_id = p.endpoint_id)
            SELECT item_id,
                   type,
                   listener_id,
                   monitored_by,
                   host_adm_status,
                   endpoint_id,
                   endpoint_adm_status
            FROM item_chain
            WHERE host_id IS NOT NULL
            LOOP
                IF r2.type = 'listener' AND r2.listener_id IS NOT NULL THEN
                    IF r2.endpoint_id IS NOT NULL AND r2.host_adm_status = 1 AND r2.endpoint_adm_status = 1 THEN
                        PERFORM log_listener_item_cache_operation(r2.item_id, 1);
                    ELSIF r2.endpoint_id IS NULL AND r2.host_adm_status = 1 THEN
                        PERFORM log_listener_item_cache_operation(r2.item_id, 1);
                    END IF;
                END IF;
                IF r2.endpoint_id IS NOT NULL AND r2.host_adm_status = 1 AND r2.endpoint_adm_status = 1 AND
                   r2.monitored_by IS NOT NULL THEN
                    PERFORM log_item_cache_operation(r2.monitored_by, r2.item_id, 1);
                ELSIF r2.endpoint_id IS NULL AND r2.host_adm_status = 1 AND r2.monitored_by IS NOT NULL THEN
                    PERFORM log_item_cache_operation(r2.monitored_by, r2.item_id, 1);
                END IF;
            END LOOP;
    END IF;

    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_log_item_cluster_changes
    AFTER INSERT OR UPDATE OR DELETE
    ON items
    FOR EACH ROW
EXECUTE FUNCTION log_item_cluster_changes();

CREATE OR REPLACE FUNCTION log_item_processors_cluster_changes()
    RETURNS TRIGGER AS
$$
DECLARE
    r2        RECORD;
    p_item_id UUID;
BEGIN
    IF TG_OP = 'INSERT' THEN
        p_item_id = NEW.item_id;
    ELSIF TG_OP = 'DELETE' THEN
        p_item_id = OLD.item_id;
    END IF;
    FOR r2 IN
        WITH RECURSIVE item_chain AS (SELECT i.item_id,
                                             i.type,
                                             i.listener_id,
                                             ht.host_id,
                                             h.monitored_by,
                                             h.adm_status  as host_adm_status,
                                             i.endpoint_id,
                                             he.adm_status as endpoint_adm_status
                                      FROM items i
                                               JOIN host_templates ht on ht.host_template_id = i.host_template_id
                                               LEFT JOIN hosts h on h.host_id = ht.host_id
                                               LEFT JOIN host_endpoints he on he.endpoint_id = i.endpoint_id
                                      WHERE i.item_id = p_item_id
                                      UNION ALL
                                      SELECT p.item_id,
                                             p.type,
                                             p.listener_id,
                                             ht.host_id,
                                             h.monitored_by,
                                             h.adm_status  as host_adm_status,
                                             p.endpoint_id,
                                             he.adm_status as endpoint_adm_status
                                      FROM item_chain c
                                               JOIN items p on p.template_item_id = c.item_id
                                               JOIN host_templates ht on ht.host_template_id = p.host_template_id
                                               LEFT JOIN hosts h on h.host_id = ht.host_id
                                               LEFT JOIN host_endpoints he on he.endpoint_id = p.endpoint_id)
        SELECT item_id,
               type,
               listener_id,
               monitored_by,
               host_adm_status,
               endpoint_id,
               endpoint_adm_status
        FROM item_chain
        WHERE host_id IS NOT NULL
        LOOP
            IF r2.type = 'listener' AND r2.listener_id IS NOT NULL THEN
                IF r2.endpoint_id IS NOT NULL AND r2.host_adm_status = 1 AND r2.endpoint_adm_status = 1 THEN
                    PERFORM log_listener_item_cache_operation(r2.item_id, 1);
                ELSIF r2.endpoint_id IS NULL AND r2.host_adm_status = 1 THEN
                    PERFORM log_listener_item_cache_operation(r2.item_id, 1);
                END IF;
            END IF;
            IF r2.endpoint_id IS NOT NULL AND r2.host_adm_status = 1 AND r2.endpoint_adm_status = 1 AND
               r2.monitored_by IS NOT NULL THEN
                PERFORM log_item_cache_operation(r2.monitored_by, r2.item_id, 1);
            ELSIF r2.endpoint_id IS NULL AND r2.host_adm_status = 1 AND r2.monitored_by IS NOT NULL THEN
                PERFORM log_item_cache_operation(r2.monitored_by, r2.item_id, 1);
            END IF;
        END LOOP;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_log_item_processors_cluster_changes
    AFTER INSERT OR DELETE
    ON item_processors
    FOR EACH ROW
EXECUTE FUNCTION log_item_processors_cluster_changes();

CREATE TABLE sinks
(
    sink_id          UUID PRIMARY KEY,
    name             TEXT        NOT NULL,
    type             TEXT        NOT NULL,
    adm_status       SMALLINT    NOT NULL,
    labels           JSONB,
    last_updated     TIMESTAMPTZ NOT NULL,
    worker_count     INTEGER     NOT NULL DEFAULT 1,
    batch_size       INTEGER     NOT NULL DEFAULT 1000,
    batch_timeout_ms INTEGER     NOT NULL DEFAULT 1000,
    channel          JSONB       NOT NULL DEFAULT '{"type": "memory"}'::JSONB,
    config           JSONB       NOT NULL,
    retry            JSONB       NOT NULL DEFAULT '{}'::JSONB,
    CONSTRAINT valid_worker_count CHECK ( worker_count >= 1 ),
    CONSTRAINT valid_batch_size CHECK ( batch_size >= 1 ),
    CONSTRAINT valid_batch_timeout_ms CHECK ( batch_timeout_ms >= 1 )
);
CREATE INDEX idx_sinks_1 ON sinks USING GIN (name gin_trgm_ops);
CREATE INDEX idx_sinks_2 ON sinks USING GIN (labels);
CREATE INDEX idx_sinks_3 ON sinks (adm_status);
CREATE INDEX idx_sinks_4 ON sinks (last_updated);

CREATE TABLE sink_inputs
(
    sink_id  UUID NOT NULL REFERENCES sinks (sink_id),
    input_id UUID NOT NULL REFERENCES processors (processor_id),
    PRIMARY KEY (sink_id, input_id)
);
CREATE INDEX idx_sink_inputs_1 ON sink_inputs (sink_id);
CREATE INDEX idx_sink_inputs_2 ON sink_inputs (input_id);

CREATE TRIGGER sink_inputs_prevent_update
    BEFORE UPDATE
    on sink_inputs
    FOR EACH ROW
EXECUTE FUNCTION prevent_update_trigger();

CREATE OR REPLACE FUNCTION log_sink_cluster_changes()
    RETURNS TRIGGER AS
$$
DECLARE
    sink_adm_status SMALLINT;
    p_sink_id       UUID;
BEGIN
    IF TG_TABLE_NAME = 'sinks' THEN
        IF TG_OP = 'DELETE' AND OLD.adm_status = 1 THEN
            INSERT INTO cluster_logs (node_id, resource, resource_id, op)
            SELECT node_id, 'sink', OLD.sink_id, 2
            FROM cluster_nodes;
        ELSIF TG_OP = 'UPDATE' AND OLD.adm_status = 1 AND NEW.adm_status <> 1 THEN
            INSERT INTO cluster_logs (node_id, resource, resource_id, op)
            SELECT node_id, 'sink', NEW.sink_id, 2
            FROM cluster_nodes;
        ELSIF TG_OP IN ('INSERT', 'UPDATE') AND NEW.adm_status = 1 THEN
            INSERT INTO cluster_logs (node_id, resource, resource_id, op)
            SELECT node_id, 'sink', NEW.sink_id, 1
            FROM cluster_nodes;
        END IF;
    END IF;
    IF TG_TABLE_NAME = 'sink_inputs' THEN
        IF TG_OP = 'DELETE' THEN
            p_sink_id = OLD.sink_id;
        ELSE
            p_sink_id = NEW.sink_id;
        END IF;
        SELECT adm_status INTO sink_adm_status FROM sinks WHERE sink_id = p_sink_id;
        IF sink_adm_status = 1 THEN
            INSERT INTO cluster_logs (node_id, resource, resource_id, op)
            SELECT node_id, 'sink', p_sink_id, 1
            FROM cluster_nodes;
        END IF;
    END IF;

    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_log_sink_cluster_changes
    AFTER INSERT OR UPDATE OR DELETE
    ON sinks
    FOR EACH ROW
EXECUTE FUNCTION log_sink_cluster_changes();

CREATE TRIGGER trg_log_sink_input_cluster_changes
    AFTER INSERT OR UPDATE OR DELETE
    ON sink_inputs
    FOR EACH ROW
EXECUTE FUNCTION log_sink_cluster_changes();

CREATE TABLE configs
(
    key          TEXT PRIMARY KEY,
    val          JSONB       NOT NULL,
    last_updated TIMESTAMPTZ NOT NULL
);

INSERT INTO configs
VALUES ('monitor', '{
  "worker_count": 100,
  "batch_size": 1000,
  "batch_timeout": 1000
}', now());

CREATE TABLE lookup_tables
(
    lookup_table_id UUID PRIMARY KEY,
    name            TEXT        NOT NULL,
    type            TEXT        NOT NULL,
    adm_status      SMALLINT    NOT NULL,
    labels          JSONB,
    last_updated    TIMESTAMPTZ NOT NULL,
    source          JSONB       NOT NULL,
    key_type        TEXT        NOT NULL,
    key             JSONB       NOT NULL
);
CREATE INDEX idx_lookup_tables_1 ON lookup_tables USING GIN (name gin_trgm_ops);
CREATE INDEX idx_lookup_tables_2 ON lookup_tables USING GIN (labels);
CREATE INDEX idx_lookup_tables_3 ON lookup_tables (adm_status);
CREATE INDEX idx_lookup_tables_4 ON lookup_tables (last_updated);

CREATE OR REPLACE FUNCTION log_lookup_table_cluster_changes()
    RETURNS TRIGGER AS
$$
BEGIN
    IF TG_OP = 'DELETE' AND OLD.adm_status = 1 THEN
        INSERT INTO cluster_logs (node_id, resource, resource_id, op)
        SELECT node_id, 'lookup_table', OLD.lookup_table_id, 2
        FROM cluster_nodes;
    ELSIF TG_OP = 'UPDATE' AND OLD.adm_status = 1 AND NEW.adm_status <> 1 THEN
        INSERT INTO cluster_logs (node_id, resource, resource_id, op)
        SELECT node_id, 'lookup_table', NEW.lookup_table_id, 2
        FROM cluster_nodes;
    ELSIF TG_OP IN ('INSERT', 'UPDATE') AND NEW.adm_status = 1 THEN
        INSERT INTO cluster_logs (node_id, resource, resource_id, op)
        SELECT node_id, 'lookup_table', NEW.lookup_table_id, 1
        FROM cluster_nodes;
    END IF;

    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_log_lookup_table_cluster_changes
    AFTER INSERT OR UPDATE OR DELETE
    ON lookup_tables
    FOR EACH ROW
EXECUTE FUNCTION log_lookup_table_cluster_changes();

CREATE TABLE pipelines
(
    pipeline_id   UUID PRIMARY KEY,
    name          TEXT        NOT NULL,
    visibility    SMALLINT    NOT NULL,
    owner_user_id UUID REFERENCES users (user_id) ON DELETE SET NULL,
    adm_status    SMALLINT    NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_updated  TIMESTAMPTZ NOT NULL,
    CHECK (owner_user_id IS NOT NULL OR visibility = 1)
);
CREATE INDEX idx_pipelines_1 ON pipelines USING GIN (name gin_trgm_ops);
CREATE INDEX idx_pipelines_2 ON pipelines (owner_user_id);
CREATE INDEX idx_pipelines_3 ON pipelines (last_updated);

CREATE TABLE pipeline_hosts
(
    pipeline_id UUID NOT NULL REFERENCES pipelines (pipeline_id) ON DELETE CASCADE,
    host_id     UUID NOT NULL REFERENCES hosts (host_id) ON DELETE CASCADE,
    PRIMARY KEY (pipeline_id, host_id)
);
CREATE INDEX idx_pipeline_hosts_1 ON pipeline_hosts (host_id);

CREATE TABLE pipeline_sinks
(
    pipeline_id UUID NOT NULL REFERENCES pipelines (pipeline_id) ON DELETE CASCADE,
    sink_id     UUID NOT NULL REFERENCES sinks (sink_id) ON DELETE CASCADE,
    PRIMARY KEY (pipeline_id, sink_id)
);
CREATE INDEX idx_pipeline_sinks_1 ON pipeline_sinks (sink_id);

CREATE TABLE pipeline_canvases
(
    canvas_id    UUID PRIMARY KEY,
    pipeline_id  UUID        NOT NULL REFERENCES pipelines (pipeline_id) ON DELETE CASCADE,
    last_updated TIMESTAMPTZ NOT NULL
);
CREATE UNIQUE INDEX idx_pipeline_canvases_1 ON pipeline_canvases (pipeline_id);

CREATE TABLE pipeline_nodes
(
    node_id      UUID PRIMARY KEY,
    canvas_id    UUID        NOT NULL REFERENCES pipeline_canvases (canvas_id) ON DELETE CASCADE,
    kind         TEXT        NOT NULL CHECK (kind IN ('source', 'processor_group', 'sink')),
    UNIQUE (canvas_id, node_id),
    UNIQUE (node_id, kind),

    position_x   INT         NOT NULL,
    position_y   INT         NOT NULL,
    last_updated TIMESTAMPTZ NOT NULL
);
CREATE UNIQUE INDEX idx_pipeline_nodes_1 ON pipeline_nodes (canvas_id) WHERE kind = 'source';

-- The processor ids in the node tables have no ON DELETE on purpose, so a
-- processor still used by a canvas cannot be deleted. The API deletes the nodes
-- before their processors.
CREATE TABLE pipeline_node_sources
(
    node_id             UUID PRIMARY KEY,
    kind                TEXT GENERATED ALWAYS AS ('source') STORED,
    FOREIGN KEY (node_id, kind) REFERENCES pipeline_nodes (node_id, kind) ON DELETE CASCADE,

    source_processor_id UUID NOT NULL UNIQUE REFERENCES processors (processor_id),
    gate_processor_id   UUID NOT NULL UNIQUE REFERENCES processors (processor_id)
);

CREATE TABLE pipeline_node_source_items
(
    node_id UUID NOT NULL REFERENCES pipeline_node_sources (node_id) ON DELETE CASCADE,
    item_id UUID NOT NULL REFERENCES items (item_id) ON DELETE CASCADE,
    PRIMARY KEY (node_id, item_id)
);
CREATE INDEX idx_pipeline_node_source_items_1 ON pipeline_node_source_items (item_id);

CREATE TABLE pipeline_node_processor_groups
(
    node_id UUID PRIMARY KEY,
    kind    TEXT GENERATED ALWAYS AS ('processor_group') STORED,
    FOREIGN KEY (node_id, kind) REFERENCES pipeline_nodes (node_id, kind) ON DELETE CASCADE
);

CREATE TABLE pipeline_node_processors
(
    node_id      UUID NOT NULL REFERENCES pipeline_node_processor_groups (node_id) ON DELETE CASCADE,
    processor_id UUID PRIMARY KEY REFERENCES processors (processor_id),
    position     INT  NOT NULL,
    UNIQUE (node_id, position)
);

CREATE TABLE pipeline_node_sinks
(
    node_id UUID PRIMARY KEY,
    kind    TEXT GENERATED ALWAYS AS ('sink') STORED,
    FOREIGN KEY (node_id, kind) REFERENCES pipeline_nodes (node_id, kind) ON DELETE CASCADE,

    sink_id UUID NOT NULL REFERENCES sinks (sink_id) ON DELETE CASCADE
);
CREATE INDEX idx_pipeline_node_sinks_1 ON pipeline_node_sinks (sink_id);

CREATE TABLE pipeline_node_relations
(
    canvas_id    UUID NOT NULL,
    from_node_id UUID NOT NULL,
    to_node_id   UUID NOT NULL,
    PRIMARY KEY (from_node_id, to_node_id),
    FOREIGN KEY (canvas_id, from_node_id)
        REFERENCES pipeline_nodes (canvas_id, node_id) ON DELETE CASCADE,
    FOREIGN KEY (canvas_id, to_node_id)
        REFERENCES pipeline_nodes (canvas_id, node_id) ON DELETE CASCADE,
    CHECK (from_node_id != to_node_id)
);
CREATE INDEX idx_pipeline_node_relations_1 ON pipeline_node_relations (to_node_id);

CREATE OR REPLACE FUNCTION pipeline_orphan_owner()
    RETURNS TRIGGER AS
$$
BEGIN
    UPDATE pipelines
    SET visibility    = 1,
        owner_user_id = NULL
    WHERE owner_user_id = OLD.user_id;
    RETURN OLD;
END;
$$ LANGUAGE plpgsql;

-- Must be BEFORE DELETE. Postgres runs the ON DELETE SET NULL on
-- pipelines.owner_user_id as an AFTER trigger. If that ran first,
-- it would leave a private pipeline with no owner, and the CHECK on
-- pipelines would reject the delete.
CREATE TRIGGER trg_pipeline_orphan_owner
    BEFORE DELETE
    ON users
    FOR EACH ROW
EXECUTE FUNCTION pipeline_orphan_owner();
