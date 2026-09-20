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

#[sqlx::test(migrations = "./migrations")]
async fn session_sources_distinguish_local_test_sessions(pool: PgPool) -> sqlx::Result<()> {
    let user = insert_user(&pool, "fixture").await?;
    let family = Uuid::new_v4();
    sqlx::query(
        "insert into sessions (
            user_id,
            refresh_token_hash,
            expires_at,
            absolute_expires_at,
            session_family_id,
            auth_source
         ) values (
            $1,
            'fixture-refresh',
            now() + interval '1 day',
            now() + interval '1 day',
            $2,
            'local_test'
         )",
    )
    .bind(user)
    .bind(family)
    .execute(&pool)
    .await?;

    let source = sqlx::query_scalar::<_, String>(
        "select auth_source from sessions where user_id = $1 and refresh_token_hash = 'fixture-refresh'",
    )
    .bind(user)
    .fetch_one(&pool)
    .await?;
    let invalid_result = sqlx::query(
        "insert into sessions (
            user_id,
            refresh_token_hash,
            expires_at,
            absolute_expires_at,
            session_family_id,
            auth_source
         ) values (
            $1,
            'invalid-refresh',
            now() + interval '1 day',
            now() + interval '1 day',
            $2,
            'unknown'
         )",
    )
    .bind(user)
    .bind(Uuid::new_v4())
    .execute(&pool)
    .await;
    let unsafe_legacy_result = sqlx::query(
        "insert into sessions (
            user_id,
            refresh_token_hash,
            expires_at,
            absolute_expires_at,
            session_family_id,
            auth_source
         ) values (
            $1,
            'unsafe-legacy-refresh',
            now() + interval '1 day',
            now() + interval '1 day',
            $2,
            'legacy'
         )",
    )
    .bind(user)
    .bind(Uuid::new_v4())
    .execute(&pool)
    .await;

    assert_eq!(source, "local_test");
    assert!(invalid_result.is_err());
    assert!(unsafe_legacy_result.is_err());
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn auth_source_repair_revokes_only_legacy_local_classifications(
    pool: PgPool,
) -> sqlx::Result<()> {
    sqlx::raw_sql(
        "create table users (
             id uuid primary key
         );
         create table identities (
             user_id uuid not null,
             provider text not null,
             provider_user_id text not null
         );
         create table sessions (
             id uuid primary key,
             user_id uuid not null,
             refresh_token_hash text not null,
             created_at timestamptz not null,
             expires_at timestamptz not null,
             revoked_at timestamptz null
         );
         create table _sqlx_migrations (
             version bigint primary key,
             installed_on timestamptz not null,
             success boolean not null
         );",
    )
    .execute(&pool)
    .await?;
    let dev_user = Uuid::new_v4();
    let steam_user = Uuid::new_v4();
    let legacy_dev_session = Uuid::new_v4();
    let steam_session = Uuid::new_v4();
    let modern_dev_session = Uuid::new_v4();
    sqlx::query("insert into users (id) values ($1), ($2)")
        .bind(dev_user)
        .bind(steam_user)
        .execute(&pool)
        .await?;
    sqlx::query(
        "insert into identities (user_id, provider, provider_user_id)
         values ($1, 'dev', 'local:alice')",
    )
    .bind(dev_user)
    .execute(&pool)
    .await?;
    sqlx::query(
        "insert into sessions (id, user_id, refresh_token_hash, created_at, expires_at)
         values (
             $1,
             $2,
             'legacy-dev-refresh',
             '2026-01-01 00:00:00+00',
             '2026-02-01 00:00:00+00'
         ), (
             $3,
             $4,
             'steam-refresh',
             '2026-01-01 00:00:00+00',
             '2026-02-01 00:00:00+00'
         )",
    )
    .bind(legacy_dev_session)
    .bind(dev_user)
    .bind(steam_session)
    .bind(steam_user)
    .execute(&pool)
    .await?;

    sqlx::raw_sql(include_str!(
        "../migrations/20260907009000_local_test_session_source.sql"
    ))
    .execute(&pool)
    .await?;

    sqlx::query(
        "insert into sessions (
            id,
            user_id,
            refresh_token_hash,
            created_at,
            expires_at,
            auth_source
         ) values (
            $1,
            $2,
            'modern-dev-refresh',
            '2026-01-03 00:00:00+00',
            '2026-02-01 00:00:00+00',
            'local_test'
         )",
    )
    .bind(modern_dev_session)
    .bind(dev_user)
    .execute(&pool)
    .await?;
    sqlx::query(
        "insert into _sqlx_migrations (version, installed_on, success)
         values (20260907009000, '2026-01-02 00:00:00+00', true)",
    )
    .execute(&pool)
    .await?;
    sqlx::raw_sql(include_str!(
        "../migrations/20260907011000_auth_source_legacy_repair.sql"
    ))
    .execute(&pool)
    .await?;

    let legacy_dev = sqlx::query_as::<_, (String, bool)>(
        "select auth_source, revoked_at is not null from sessions where id = $1",
    )
    .bind(legacy_dev_session)
    .fetch_one(&pool)
    .await?;
    let steam = sqlx::query_as::<_, (String, bool)>(
        "select auth_source, revoked_at is not null from sessions where id = $1",
    )
    .bind(steam_session)
    .fetch_one(&pool)
    .await?;
    let modern_dev = sqlx::query_as::<_, (String, bool)>(
        "select auth_source, revoked_at is not null from sessions where id = $1",
    )
    .bind(modern_dev_session)
    .fetch_one(&pool)
    .await?;

    assert_eq!(legacy_dev, ("legacy".to_string(), true));
    assert_eq!(steam, ("steam".to_string(), false));
    assert_eq!(modern_dev, ("local_test".to_string(), false));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn session_refresh_security_migration_backfills_families_and_invalidates_duplicates(
    pool: PgPool,
) -> sqlx::Result<()> {
    sqlx::raw_sql(
        "create table sessions (
             id uuid primary key,
             user_id uuid not null,
             refresh_token_hash text not null,
             created_at timestamptz not null,
             expires_at timestamptz not null,
             revoked_at timestamptz null
         );
         create table steam_login_challenges (
             expires_at timestamptz not null
         );",
    )
    .execute(&pool)
    .await?;
    let session = Uuid::new_v4();
    let duplicate_first = Uuid::new_v4();
    let duplicate_second = Uuid::new_v4();
    sqlx::query(
        "insert into sessions (id, user_id, refresh_token_hash, created_at, expires_at)
         values (
             $1,
             $2,
             'unique-refresh',
             now() - interval '1 hour',
             now() + interval '1 day'
         ), (
             $3,
             $2,
             'duplicated-refresh',
             now() - interval '1 hour',
             now() + interval '1 day'
         ), (
             $4,
             $2,
             'duplicated-refresh',
             now() - interval '1 hour',
             now() + interval '1 day'
         )",
    )
    .bind(session)
    .bind(Uuid::new_v4())
    .bind(duplicate_first)
    .bind(duplicate_second)
    .execute(&pool)
    .await?;

    sqlx::raw_sql(include_str!(
        "../migrations/20260907012000_session_refresh_security.sql"
    ))
    .execute(&pool)
    .await?;

    let backfilled = sqlx::query_scalar::<_, bool>(
        "select
             session_family_id = id
             and absolute_expires_at = expires_at
             and refresh_token_used_at is null
         from sessions
         where id = $1",
    )
    .bind(session)
    .fetch_one(&pool)
    .await?;
    let duplicate_active = sqlx::query_scalar::<_, i64>(
        "select count(*)
         from sessions
         where id = any($1) and revoked_at is null",
    )
    .bind(vec![duplicate_first, duplicate_second])
    .fetch_one(&pool)
    .await?;
    let duplicate_hashes_are_distinct = sqlx::query_scalar::<_, bool>(
        "select count(distinct refresh_token_hash) = 2
         from sessions
         where id = any($1)",
    )
    .bind(vec![duplicate_first, duplicate_second])
    .fetch_one(&pool)
    .await?;

    assert!(backfilled);
    assert_eq!(duplicate_active, 0);
    assert!(duplicate_hashes_are_distinct);
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
