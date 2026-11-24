// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! What `db::Pool` counts, against a real Postgres: checkouts, the wait for a
//! connection, how long it was held, and the connections taken out for good.

mod setup;

use anyhow::ensure;
use setup::eventually;
use std::time::Duration;

#[tokio::test]
async fn checkouts_waits_and_holds_are_counted() {
    setup::setup_logging();
    let (test_db, _) = setup::setup_db().await;
    // one connection, so a second checkout has to wait for the first
    let pool = setup::pool(&test_db.url, 1);

    let idle = pool.snapshot();
    assert_eq!((idle.max_size, idle.size, idle.checkouts), (1, 0, 0));

    let client = pool.get().await.unwrap();
    client.query_one("SELECT 1", &[]).await.unwrap();
    let held = pool.snapshot();
    assert_eq!((held.size, held.available, held.waiting), (1, 0, 0));
    assert_eq!((held.checkouts, held.connections_created), (1, 1));
    assert_eq!(held.hold_time, Duration::ZERO, "counted on return");

    let waiter = tokio::spawn({
        let pool = pool.clone();
        async move { pool.get().await.map(drop) }
    });
    eventually(async || {
        ensure!(pool.snapshot().waiting == 1, "nobody is waiting yet");
        Ok(())
    })
    .await;
    drop(client);
    waiter.await.unwrap().unwrap();

    let released = pool.snapshot();
    assert_eq!((released.size, released.available), (1, 1));
    assert_eq!((released.waiting, released.checkouts), (0, 2));
    assert_eq!(released.connections_created, 1, "the connection is reused");
    assert_eq!(released.checkout_errors, 0);
    assert!(released.checkout_wait_time > Duration::ZERO);
    assert!(released.hold_time > Duration::ZERO);
}

#[tokio::test]
async fn a_detached_connection_leaves_the_pool() {
    setup::setup_logging();
    let (test_db, _) = setup::setup_db().await;
    let pool = setup::pool(&test_db.url, 1);

    let detached = pool.get().await.unwrap().detach();
    let after = pool.snapshot();
    assert_eq!((after.size, after.available, after.checkouts), (0, 0, 1));

    // its slot is free again while it stays usable
    let client = pool.get().await.unwrap();
    detached.query_one("SELECT 1", &[]).await.unwrap();
    drop(client);
    assert_eq!(pool.snapshot().connections_created, 2);
}

#[tokio::test]
async fn a_failed_checkout_is_counted() {
    setup::setup_logging();
    let (test_db, _) = setup::setup_db().await;
    let pool = setup::pool(&test_db.url, 1);

    pool.close();
    assert!(pool.get().await.is_err());
    let closed = pool.snapshot();
    assert_eq!((closed.checkouts, closed.checkout_errors), (0, 1));
}
