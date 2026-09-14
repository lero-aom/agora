#![cfg(feature = "postgres-tests")]

use std::sync::Arc;

use sqlx::{PgPool, Row};
use tokio::sync::Barrier;
use uuid::Uuid;

#[sqlx::test(migrations = "./migrations")]
async fn block_insert_removes_active_friendship(pool: PgPool) -> sqlx::Result<()> {
    let (first, second) = insert_users(&pool).await?;
    sqlx::query(
        "insert into friendships (requester_id, addressee_id, status)
         values ($1, $2, 'accepted')",
    )
    .bind(first)
    .bind(second)
    .execute(&pool)
    .await?;

    sqlx::query("insert into blocks (blocker_id, blocked_id) values ($1, $2)")
        .bind(first)
        .bind(second)
        .execute(&pool)
        .await?;

    let status = sqlx::query_scalar::<_, String>(
        "select status
         from friendships
         where requester_id = $1 and addressee_id = $2",
    )
    .bind(first)
    .bind(second)
    .fetch_one(&pool)
    .await?;

    assert_eq!(status, "removed");
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn active_friendship_is_rejected_when_block_exists(pool: PgPool) -> sqlx::Result<()> {
    let (first, second) = insert_users(&pool).await?;
    sqlx::query("insert into blocks (blocker_id, blocked_id) values ($1, $2)")
        .bind(first)
        .bind(second)
        .execute(&pool)
        .await?;

    let result = sqlx::query(
        "insert into friendships (requester_id, addressee_id, status)
         values ($1, $2, 'pending')",
    )
    .bind(first)
    .bind(second)
    .execute(&pool)
    .await;

    assert!(result.is_err());
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn concurrent_friendship_and_block_cannot_leave_active_friendship(
    pool: PgPool,
) -> sqlx::Result<()> {
    let (first, second) = insert_users(&pool).await?;
    let barrier = Arc::new(Barrier::new(2));

    let friendship_pool = pool.clone();
    let friendship_barrier = barrier.clone();
    let friendship_task = tokio::spawn(async move {
        friendship_barrier.wait().await;
        sqlx::query(
            "insert into friendships (requester_id, addressee_id, status)
             values ($1, $2, 'pending')",
        )
        .bind(first)
        .bind(second)
        .execute(&friendship_pool)
        .await
        .map(|_| ())
    });

    let block_pool = pool.clone();
    let block_barrier = barrier.clone();
    let block_task = tokio::spawn(async move {
        block_barrier.wait().await;
        sqlx::query("insert into blocks (blocker_id, blocked_id) values ($1, $2)")
            .bind(first)
            .bind(second)
            .execute(&block_pool)
            .await
            .map(|_| ())
    });

    let _ = friendship_task.await.expect("friendship task panicked");
    let _ = block_task.await.expect("block task panicked");

    let active_friendships = sqlx::query_scalar::<_, i64>(
        "select count(*)
         from friendships
         where status in ('pending', 'accepted')
           and ((requester_id = $1 and addressee_id = $2)
             or (requester_id = $2 and addressee_id = $1))",
    )
    .bind(first)
    .bind(second)
    .fetch_one(&pool)
    .await?;
    let blocks = sqlx::query_scalar::<_, i64>(
        "select count(*)
         from blocks
         where blocker_id = $1 and blocked_id = $2",
    )
    .bind(first)
    .bind(second)
    .fetch_one(&pool)
    .await?;

    assert_eq!(blocks, 1);
    assert_eq!(active_friendships, 0);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn moderation_actions_are_immutable(pool: PgPool) -> sqlx::Result<()> {
    let (moderator, target) = insert_users(&pool).await?;
    let row = sqlx::query(
        "insert into moderation_actions (moderator_id, target_user_id, action, reason)
         values ($1, $2, 'suspend', 'spam')
         returning id",
    )
    .bind(moderator)
    .bind(target)
    .fetch_one(&pool)
    .await?;
    let action_id: Uuid = row.try_get("id")?;

    let update_result =
        sqlx::query("update moderation_actions set reason = 'edited' where id = $1")
            .bind(action_id)
            .execute(&pool)
            .await;
    let delete_result = sqlx::query("delete from moderation_actions where id = $1")
        .bind(action_id)
        .execute(&pool)
        .await;

    assert!(update_result.is_err());
    assert!(delete_result.is_err());
    Ok(())
}

async fn insert_users(pool: &PgPool) -> sqlx::Result<(Uuid, Uuid)> {
    let first = insert_user(pool, "first").await?;
    let second = insert_user(pool, "second").await?;
    Ok((first, second))
}

async fn insert_user(pool: &PgPool, name: &str) -> sqlx::Result<Uuid> {
    let row = sqlx::query("insert into users (display_name) values ($1) returning id")
        .bind(name)
        .fetch_one(pool)
        .await?;
    row.try_get("id")
}
