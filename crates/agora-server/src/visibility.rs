use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

// Blocks are unilateral records with a bilateral communication effect.
pub(crate) fn not_blocked_between_sql(first: &str, second: &str) -> String {
    format!(
        "not exists (
            select 1
            from blocks b
            where (b.blocker_id = {first} and b.blocked_id = {second})
               or (b.blocker_id = {second} and b.blocked_id = {first})
        )"
    )
}

pub(crate) async fn blocked_between_tx(
    tx: &mut Transaction<'_, Postgres>,
    first: Uuid,
    second: Uuid,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar::<_, bool>(
        "select exists(
            select 1
            from blocks
            where (blocker_id = $1 and blocked_id = $2)
               or (blocker_id = $2 and blocked_id = $1)
        )",
    )
    .bind(first)
    .bind(second)
    .fetch_one(&mut **tx)
    .await
}

pub(crate) async fn can_deliver_between(
    db: &PgPool,
    viewer_id: Uuid,
    author_id: Uuid,
) -> Result<bool, sqlx::Error> {
    if viewer_id == author_id {
        return Ok(true);
    }

    let blocked = sqlx::query_scalar::<_, bool>(
        "select exists(
            select 1
            from blocks
            where (blocker_id = $1 and blocked_id = $2)
               or (blocker_id = $2 and blocked_id = $1)
        )",
    )
    .bind(viewer_id)
    .bind(author_id)
    .fetch_one(db)
    .await?;

    Ok(!blocked)
}
