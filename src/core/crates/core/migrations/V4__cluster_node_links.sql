-- What each main node sees when it dials each of its peers on the inter-node
-- gRPC port: one row per directed link, written by the source node. A node whose
-- inbound port is closed shows as every row with dst_node_id = that node failing,
-- while its own rows (it dials out fine) stay ok.
--
-- A row is written only when it changes, so it is only as fresh as its source:
-- trust it while src_node_id's cluster_nodes row is active with a recent
-- last_heartbeat. Rows go with either node (the leader deletes an expired one).
-- No trigger: nothing here is logged to cluster_logs.
CREATE TABLE cluster_node_links
(
    src_node_id  UUID        NOT NULL REFERENCES cluster_nodes (node_id) ON DELETE CASCADE,
    dst_node_id  UUID        NOT NULL REFERENCES cluster_nodes (node_id) ON DELETE CASCADE,
    -- the advertise address that was dialed
    dst_address  TEXT        NOT NULL,
    -- ok, version_skew (both: the link works), timeout, refused, unreachable,
    -- tls, unauthenticated, wrong_identity, error
    status       TEXT        NOT NULL,
    -- the error as text, NULL when ok
    reason       TEXT,
    -- NULL when the peer did not answer
    peer_version TEXT,
    -- when status last changed
    state_since  TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (src_node_id, dst_node_id)
);
CREATE INDEX idx_cluster_node_links_dst ON cluster_node_links (dst_node_id);
