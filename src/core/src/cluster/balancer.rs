// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Where hosts should live, decided by the leader every tick. Kept free of I/O so
//! the rules can be tested without a database: `NodeLock::leader_tasks` gathers
//! the [`NodeLoad`]s and applies the returned [`Move`]s in order.
//!
//! At most `max_active_nodes` live nodes carry hosts; the other live nodes are
//! spares. A failed node's hosts move as one block to a spare, and a node coming
//! back finds its hosts gone and becomes the spare, so nothing moves twice.
//! Sticky hosts are placed by an operator and never counted or moved here.

use std::collections::VecDeque;
use uuid::Uuid;

/// One `cluster_nodes` row as the balancer sees it.
#[derive(Debug, Clone, Copy)]
pub struct NodeLoad {
    pub node_id: Uuid,
    /// Heartbeat within the stale threshold.
    pub live: bool,
    /// Non-sticky hosts only.
    pub hosts: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Move {
    /// Every non-sticky host of a failed node goes to one spare.
    Block { from: Uuid, to: Uuid },
    /// Marker, no hosts move by itself: a failed node with no spare to take it,
    /// whose `hosts` are spread by the [`Move::Transfer`]s that follow.
    Release { from: Uuid, hosts: i64 },
    /// Marker, no hosts move by itself: a live node over the active limit, emptied
    /// by the [`Move::Transfer`]s that follow so it becomes a spare.
    Evacuate { from: Uuid, hosts: i64 },
    /// `count` non-sticky hosts from `from` (`None`: unassigned hosts) to `to`.
    /// Hosts always go straight to their new owner, never through `NULL`: each
    /// change of `monitored_by` makes the hosts trigger log to every node.
    Transfer {
        from: Option<Uuid>,
        to: Uuid,
        count: i64,
    },
}

/// `max_active == 0` means every live node may carry hosts. Ties are broken by
/// `node_id` so repeated ticks agree on which node is the spare.
pub fn plan(nodes: &[NodeLoad], unassigned: i64, max_active: usize) -> Vec<Move> {
    let mut moves = Vec::new();
    let mut nodes = nodes.to_vec();
    nodes.sort_by_key(|n| n.node_id);
    let live = nodes.iter().filter(|n| n.live).count();
    if live == 0 {
        return moves;
    }
    let target = if max_active == 0 {
        live
    } else {
        max_active.min(live)
    };
    let mut active: Vec<(Uuid, i64)> = nodes
        .iter()
        .filter(|n| n.live && n.hosts > 0)
        .map(|n| (n.node_id, n.hosts))
        .collect();
    let mut spares: VecDeque<Uuid> = nodes
        .iter()
        .filter(|n| n.live && n.hosts == 0)
        .map(|n| n.node_id)
        .collect();
    // hosts that must find a new owner in the active set, drained in order
    let mut sources: Vec<(Option<Uuid>, i64)> = Vec::new();

    for dead in nodes.iter().filter(|n| !n.live && n.hosts > 0) {
        if active.len() < target
            && let Some(spare) = spares.pop_front()
        {
            moves.push(Move::Block {
                from: dead.node_id,
                to: spare,
            });
            active.push((spare, dead.hosts));
        } else {
            moves.push(Move::Release {
                from: dead.node_id,
                hosts: dead.hosts,
            });
            sources.push((Some(dead.node_id), dead.hosts));
        }
    }

    // only a lowered limit gets here, since failures only ever shrink `active`;
    // one node per tick, the one whose evacuation moves the fewest hosts
    if active.len() > target
        && let Some(idx) = active
            .iter()
            .enumerate()
            .min_by_key(|(_, (id, hosts))| (*hosts, *id))
            .map(|(idx, _)| idx)
    {
        let (from, hosts) = active.remove(idx);
        moves.push(Move::Evacuate { from, hosts });
        sources.push((Some(from), hosts));
    }

    while active.len() < target
        && let Some(spare) = spares.pop_front()
    {
        active.push((spare, 0));
    }
    if unassigned > 0 {
        sources.push((None, unassigned));
    }
    if active.is_empty() {
        return moves;
    }

    let n = active.len() as i64;
    let pending: i64 = sources.iter().map(|(_, hosts)| hosts).sum();
    let total = active.iter().map(|(_, hosts)| hosts).sum::<i64>() + pending;
    let ideal = (total + n - 1) / n;

    // `ideal` is rounded up, so the deficits below it always cover every source
    let mut src = 0;
    for (to, hosts) in active.iter_mut() {
        while src < sources.len() && *hosts < ideal {
            let (from, left) = &mut sources[src];
            let count = (ideal - *hosts).min(*left);
            moves.push(Move::Transfer {
                from: *from,
                to: *to,
                count,
            });
            *hosts += count;
            *left -= count;
            if *left == 0 {
                src += 1;
            }
        }
    }

    for i in 0..active.len() {
        for j in 0..active.len() {
            let excess = active[i].1 - ideal;
            if excess <= 0 {
                break;
            }
            let deficit = ideal - active[j].1;
            if deficit <= 0 {
                continue;
            }
            let count = excess.min(deficit);
            moves.push(Move::Transfer {
                from: Some(active[i].0),
                to: active[j].0,
                count,
            });
            active[i].1 -= count;
            active[j].1 += count;
        }
    }
    moves
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustc_hash::FxHashMap;

    fn ids(n: usize) -> Vec<Uuid> {
        let mut ids: Vec<Uuid> = (0..n).map(|_| Uuid::now_v7()).collect();
        ids.sort();
        ids
    }

    fn load(node_id: Uuid, live: bool, hosts: i64) -> NodeLoad {
        NodeLoad {
            node_id,
            live,
            hosts,
        }
    }

    /// Host count per node after applying `moves`, dropping empty nodes.
    fn apply(nodes: &[NodeLoad], unassigned: i64, moves: &[Move]) -> FxHashMap<Uuid, i64> {
        let mut counts: FxHashMap<Uuid, i64> = nodes.iter().map(|n| (n.node_id, n.hosts)).collect();
        let mut pool = unassigned;
        for mv in moves {
            match *mv {
                Move::Block { from, to } => {
                    let hosts = counts.insert(from, 0).unwrap();
                    *counts.get_mut(&to).unwrap() += hosts;
                }
                Move::Release { .. } | Move::Evacuate { .. } => {}
                Move::Transfer { from, to, count } => {
                    match from {
                        None => pool -= count,
                        Some(from) => *counts.get_mut(&from).unwrap() -= count,
                    }
                    *counts.get_mut(&to).unwrap() += count;
                }
            }
        }
        assert_eq!(pool, 0, "every unassigned host is placed");
        for mv in moves {
            if let Move::Release { from, .. } | Move::Evacuate { from, .. } = mv {
                assert_eq!(counts[from], 0, "a released or evacuated node is emptied");
            }
        }
        counts.retain(|_, hosts| {
            assert!(*hosts >= 0, "no node goes negative");
            *hosts > 0
        });
        counts
    }

    #[test]
    fn no_limit_spreads_over_every_live_node() {
        let id = ids(3);
        let nodes = [
            load(id[0], true, 0),
            load(id[1], true, 0),
            load(id[2], true, 0),
        ];
        let moves = plan(&nodes, 6, 0);
        let counts = apply(&nodes, 6, &moves);
        assert_eq!(counts.len(), 3);
        assert!(counts.values().all(|&hosts| hosts == 2));
    }

    #[test]
    fn no_limit_moves_hosts_back_to_a_returning_node() {
        // the double churn `max_active_nodes` exists to avoid, kept as the default
        let id = ids(2);
        let nodes = [load(id[0], true, 4), load(id[1], true, 0)];
        let counts = apply(&nodes, 0, &plan(&nodes, 0, 0));
        assert_eq!(counts[&id[0]], 2);
        assert_eq!(counts[&id[1]], 2);
    }

    #[test]
    fn limit_keeps_the_rest_as_spares() {
        let id = ids(3);
        let nodes = [
            load(id[0], true, 0),
            load(id[1], true, 0),
            load(id[2], true, 0),
        ];
        let counts = apply(&nodes, 6, &plan(&nodes, 6, 2));
        assert_eq!(counts.len(), 2);
        assert!(
            !counts.contains_key(&id[2]),
            "spare picked by node_id order"
        );
    }

    #[test]
    fn a_spare_takes_a_failed_node_as_one_block() {
        let id = ids(3);
        let nodes = [
            load(id[0], false, 3),
            load(id[1], true, 3),
            load(id[2], true, 0),
        ];
        let moves = plan(&nodes, 0, 2);
        assert_eq!(
            moves,
            vec![Move::Block {
                from: id[0],
                to: id[2]
            }]
        );
    }

    #[test]
    fn without_a_spare_a_failed_node_is_spread() {
        let id = ids(3);
        let nodes = [
            load(id[0], false, 4),
            load(id[1], true, 4),
            load(id[2], true, 4),
        ];
        let moves = plan(&nodes, 0, 3);
        assert_eq!(
            moves[0],
            Move::Release {
                from: id[0],
                hosts: 4
            }
        );
        assert!(
            !moves
                .iter()
                .any(|mv| matches!(mv, Move::Transfer { from: None, .. })),
            "released hosts move straight to their new owner"
        );
        let counts = apply(&nodes, 0, &moves);
        assert_eq!(counts[&id[1]], 6);
        assert_eq!(counts[&id[2]], 6);
    }

    #[test]
    fn a_returning_node_stays_a_spare() {
        let id = ids(2);
        let nodes = [load(id[0], true, 0), load(id[1], true, 4)];
        assert!(plan(&nodes, 0, 1).is_empty());
    }

    #[test]
    fn lowering_the_limit_evacuates_one_node_per_tick() {
        let id = ids(4);
        let nodes = [
            load(id[0], true, 3),
            load(id[1], true, 2),
            load(id[2], true, 3),
            load(id[3], true, 3),
        ];
        let moves = plan(&nodes, 0, 2);
        let evacuated: Vec<_> = moves
            .iter()
            .filter(|mv| matches!(mv, Move::Evacuate { .. }))
            .collect();
        assert_eq!(
            evacuated,
            vec![&Move::Evacuate {
                from: id[1],
                hosts: 2
            }]
        );
        assert!(
            moves
                .iter()
                .all(|mv| !matches!(mv, Move::Transfer { from: None, .. })),
            "evacuated hosts move straight to their new owner"
        );
        let counts = apply(&nodes, 0, &moves);
        assert_eq!(counts.len(), 3, "a second node waits for the next tick");
        let next: Vec<_> = counts.iter().map(|(&id, &h)| load(id, true, h)).collect();
        let counts = apply(&next, 0, &plan(&next, 0, 2));
        assert_eq!(counts.len(), 2);
        assert_eq!(counts.values().sum::<i64>(), 11);
    }

    #[test]
    fn raising_the_limit_activates_a_spare() {
        let id = ids(3);
        let nodes = [
            load(id[0], true, 4),
            load(id[1], true, 4),
            load(id[2], true, 0),
        ];
        let counts = apply(&nodes, 0, &plan(&nodes, 0, 3));
        assert_eq!(counts.len(), 3);
        assert_eq!(counts.values().sum::<i64>(), 8);
        assert!(counts.values().all(|&hosts| hosts <= 3));
    }

    #[test]
    fn a_limit_above_live_is_clamped() {
        let id = ids(2);
        let nodes = [load(id[0], true, 0), load(id[1], true, 0)];
        let counts = apply(&nodes, 4, &plan(&nodes, 4, 10));
        assert_eq!(counts[&id[0]], 2);
        assert_eq!(counts[&id[1]], 2);
    }

    #[test]
    fn rebalancing_never_drains_a_source_below_ideal() {
        let id = ids(3);
        let nodes = [
            load(id[0], true, 9),
            load(id[1], true, 1),
            load(id[2], true, 2),
        ];
        let counts = apply(&nodes, 0, &plan(&nodes, 0, 0));
        assert_eq!(counts[&id[0]], 4);
        assert_eq!(counts.values().sum::<i64>(), 12);
        assert!(counts.values().all(|&hosts| hosts == 4));
    }

    #[test]
    fn an_empty_cluster_activates_up_to_the_limit() {
        let id = ids(4);
        let nodes: Vec<_> = id.iter().map(|&id| load(id, true, 0)).collect();
        let counts = apply(&nodes, 7, &plan(&nodes, 7, 2));
        assert_eq!(counts.len(), 2);
        assert_eq!(counts.values().sum::<i64>(), 7);
    }

    #[test]
    fn no_live_node_plans_nothing() {
        let id = ids(1);
        assert!(plan(&[load(id[0], false, 3)], 2, 0).is_empty());
    }
}
