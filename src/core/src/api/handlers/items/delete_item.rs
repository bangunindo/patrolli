// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::api::error::ApiResultExt;
use crate::api::response::{ResponseData, ResponseError};
use crate::api::state::AppState;
use crate::db::schema::Items;
use axum::{
    extract::{Path, State},
    http::StatusCode,
};
use sea_query::{Expr, ExprTrait, PostgresQueryBuilder, Query as DbQuery};
use sea_query_postgres::PostgresBinder;
use std::sync::Arc;
use uuid::Uuid;

type DeleteItemResponse = ();

/// Delete an item by ID
#[utoipa::path(
	delete,
	path = "/items/{item_id}",
	params(
		("item_id" = Uuid, Path, description = "Item UUID")
	),
    responses(
        (status = 204, description = "Item deleted", body = ResponseData<DeleteItemResponse>),
        (status = 404, description = "Item not found", body = ResponseError),
        (status = 500, description = "Internal server error", body = ResponseError)
    ),
	tag = "items",
    security(
        ("bearerAuth" = [])
    )
)]
pub async fn delete_item(
    State(state): State<Arc<AppState>>,
    Path(item_id): Path<Uuid>,
) -> Result<ResponseData<()>, ResponseError> {
    let mut client = state.db_pool.get().await.ctx("get database connection")?;

    let tx = client.transaction().await.ctx("start transaction")?;

    // get the item
    let mut query = DbQuery::select();
    query
        .column(Items::ItemId)
        .from(Items::Table)
        .and_where(Expr::col(Items::ItemId).eq(item_id));

    let (sql, values) = query.build_postgres(PostgresQueryBuilder);

    let rows = tx
        .query(sql.as_str(), &values.as_params())
        .await
        .ctx("execute item query")?;

    let Some(_) = rows.first() else {
        return Err(ResponseError::from_status(StatusCode::NOT_FOUND)
            .with_title("Item not found")
            .with_detail(format!("No item found for item_id: {}", item_id)));
    };

    // delete the target item and all items that inherit from it
    let statement = "
		WITH RECURSIVE tree AS (
			SELECT item_id
			FROM items
			WHERE item_id = $1::uuid
			UNION ALL
			SELECT i.item_id
			FROM items i
			JOIN tree t ON t.item_id = i.template_item_id
		),
        delete_item_processors AS (
            DELETE FROM item_processors
            WHERE item_id IN (SELECT item_id FROM tree)
        )
		DELETE FROM items
		WHERE item_id IN (SELECT item_id FROM tree);
	";

    _ = tx
        .execute(statement, &[&item_id])
        .await
        .ctx("execute item delete query")?;

    tx.commit().await.ctx("commit transaction")?;

    Ok(ResponseData::from_status(StatusCode::NO_CONTENT))
}
