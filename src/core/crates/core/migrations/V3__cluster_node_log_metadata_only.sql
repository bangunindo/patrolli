-- A node's op_status flips whenever the leader marks it inactive and its next
-- heartbeat brings it back. No watcher reads it from the logs (a peer is only
-- what it connects with, the metadata), so logging those flips to every other
-- node was pure churn.
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
        AND OLD.metadata IS DISTINCT FROM NEW.metadata THEN
        INSERT INTO cluster_logs (node_id, resource, resource_id, op)
        SELECT node_id, 'cluster_node', NEW.node_id, 1
        FROM cluster_nodes
        WHERE node_id <> NEW.node_id;
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;
