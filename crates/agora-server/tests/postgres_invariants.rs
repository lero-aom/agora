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
async fn block_update_removes_active_friendship_for_new_pair(pool: PgPool) -> sqlx::Result<()> {
    let first = insert_user(&pool, "first").await?;
    let second = insert_user(&pool, "second").await?;
    let third = insert_user(&pool, "third").await?;
    sqlx::query(
        "insert into friendships (requester_id, addressee_id, status)
         values ($1, $2, 'accepted')",
    )
    .bind(first)
    .bind(third)
    .execute(&pool)
    .await?;
    sqlx::query("insert into blocks (blocker_id, blocked_id) values ($1, $2)")
        .bind(first)
        .bind(second)
        .execute(&pool)
        .await?;

    sqlx::query("update blocks set blocked_id = $3 where blocker_id = $1 and blocked_id = $2")
        .bind(first)
        .bind(second)
        .bind(third)
        .execute(&pool)
        .await?;

    let status = sqlx::query_scalar::<_, String>(
        "select status
         from friendships
         where requester_id = $1 and addressee_id = $2",
    )
    .bind(first)
    .bind(third)
    .fetch_one(&pool)
    .await?;

    assert_eq!(status, "removed");
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

#[sqlx::test(migrations = "./migrations")]
async fn open_reports_are_deduplicated_by_reporter_target_and_message(
    pool: PgPool,
) -> sqlx::Result<()> {
    let (reporter, reported) = insert_users(&pool).await?;
    sqlx::query(
        "insert into reports (reporter_id, reported_user_id, reason)
         values ($1, $2, 'spam')",
    )
    .bind(reporter)
    .bind(reported)
    .execute(&pool)
    .await?;

    let duplicate_result = sqlx::query(
        "insert into reports (reporter_id, reported_user_id, reason)
         values ($1, $2, 'spam again')",
    )
    .bind(reporter)
    .bind(reported)
    .execute(&pool)
    .await;

    assert!(duplicate_result.is_err());
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn report_upsert_returns_the_existing_open_report_id(pool: PgPool) -> sqlx::Result<()> {
    let (reporter, reported) = insert_users(&pool).await?;
    let sql = "insert into reports (
            reporter_id,
            reported_user_id,
            message_id,
            message_kind,
            reason
        ) values ($1, $2, $3, $4, $5)
        on conflict (
            reporter_id,
            reported_user_id,
            coalesce(message_id, '00000000-0000-0000-0000-000000000000'::uuid),
            coalesce(message_kind, '')
        ) where status = 'open'
        do update set id = reports.id
        returning id";
    let first = sqlx::query(sql)
        .bind(reporter)
        .bind(reported)
        .bind(Option::<Uuid>::None)
        .bind(Option::<String>::None)
        .bind("first")
        .fetch_one(&pool)
        .await?;
    let first_id: Uuid = first.try_get("id")?;
    let second = sqlx::query(sql)
        .bind(reporter)
        .bind(reported)
        .bind(Option::<Uuid>::None)
        .bind(Option::<String>::None)
        .bind("second")
        .fetch_one(&pool)
        .await?;
    let second_id: Uuid = second.try_get("id")?;

    assert_eq!(first_id, second_id);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn report_deduplication_migration_repairs_existing_open_duplicates(
    pool: PgPool,
) -> sqlx::Result<()> {
    sqlx::raw_sql(
        "create extension if not exists pgcrypto;
         create table reports (
            id uuid primary key default gen_random_uuid(),
            reporter_id uuid not null,
            reported_user_id uuid not null,
            message_id uuid null,
            message_kind text null,
            reason text not null,
            details text null,
            status text not null default 'open',
            created_at timestamptz not null default now(),
            resolved_at timestamptz null,
            resolved_by uuid null
         );",
    )
    .execute(&pool)
    .await?;
    let reporter = Uuid::new_v4();
    let reported = Uuid::new_v4();
    sqlx::query(
        "insert into reports (reporter_id, reported_user_id, reason, created_at)
         values ($1, $2, 'first', now() - interval '1 minute'), ($1, $2, 'second', now())",
    )
    .bind(reporter)
    .bind(reported)
    .execute(&pool)
    .await?;

    sqlx::raw_sql(include_str!(
        "../migrations/20260907007000_report_deduplication.sql"
    ))
    .execute(&pool)
    .await?;

    let open_reports = sqlx::query_scalar::<_, i64>(
        "select count(*)
         from reports
         where reporter_id = $1 and reported_user_id = $2 and status = 'open'",
    )
    .bind(reporter)
    .bind(reported)
    .fetch_one(&pool)
    .await?;
    let dismissed_reports = sqlx::query_scalar::<_, i64>(
        "select count(*)
         from reports
         where reporter_id = $1 and reported_user_id = $2 and status = 'dismissed'",
    )
    .bind(reporter)
    .bind(reported)
    .fetch_one(&pool)
    .await?;
    let duplicate_result = sqlx::query(
        "insert into reports (reporter_id, reported_user_id, reason)
         values ($1, $2, 'third')",
    )
    .bind(reporter)
    .bind(reported)
    .execute(&pool)
    .await;

    assert_eq!(open_reports, 1);
    assert_eq!(dismissed_reports, 1);
    assert!(duplicate_result.is_err());
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn owner_role_and_audit_report_links_are_governed(pool: PgPool) -> sqlx::Result<()> {
    let (owner, target) = insert_users(&pool).await?;
    sqlx::query("update users set role = 'owner' where id = $1")
        .bind(owner)
        .execute(&pool)
        .await?;
    let report = sqlx::query(
        "insert into reports (reporter_id, reported_user_id, reason)
         values ($1, $2, 'spam')
         returning id",
    )
    .bind(owner)
    .bind(target)
    .fetch_one(&pool)
    .await?;
    let report_id: Uuid = report.try_get("id")?;
    sqlx::query(
        "insert into moderation_actions (moderator_id, target_user_id, action, reason, report_id)
         values ($1, $2, 'resolve_report', 'reviewed', $3)",
    )
    .bind(owner)
    .bind(target)
    .bind(report_id)
    .execute(&pool)
    .await?;

    let delete_rule = sqlx::query_scalar::<_, String>(
        "select delete_rule
         from information_schema.referential_constraints
         where constraint_name = 'moderation_actions_report_id_fkey'",
    )
    .fetch_one(&pool)
    .await?;

    let delete_result = sqlx::query("delete from reports where id = $1")
        .bind(report_id)
        .execute(&pool)
        .await;

    assert_eq!(delete_rule, "RESTRICT");
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
