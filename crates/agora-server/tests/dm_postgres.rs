#![cfg(feature = "postgres-tests")]

use sqlx::{PgPool, Row};
use uuid::Uuid;

#[sqlx::test(migrations = "./migrations")]
async fn direct_threads_are_unique_pairs_with_exact_members(pool: PgPool) -> sqlx::Result<()> {
    let first = insert_user(&pool, "first").await?;
    let second = insert_user(&pool, "second").await?;
    let third = insert_user(&pool, "third").await?;
    let thread_id = insert_direct_thread(&pool, first, second).await?;

    let extra_member = sqlx::query("insert into dm_members (thread_id, user_id) values ($1, $2)")
        .bind(thread_id)
        .bind(third)
        .execute(&pool)
        .await;

    let malformed_thread = sqlx::query("insert into dm_threads default values returning id")
        .fetch_one(&pool)
        .await?;
    let malformed_thread_id: Uuid = malformed_thread.try_get("id")?;
    sqlx::query(
        "insert into dm_members (thread_id, user_id)
         values ($1, $2), ($1, $3), ($1, $4)",
    )
    .bind(malformed_thread_id)
    .bind(first)
    .bind(second)
    .bind(third)
    .execute(&pool)
    .await?;
    let malformed_direct_thread = sqlx::query(
        "insert into dm_direct_threads (thread_id, first_user_id, second_user_id)
         values ($1, $2, $3)",
    )
    .bind(malformed_thread_id)
    .bind(first.min(second))
    .bind(first.max(second))
    .execute(&pool)
    .await;

    let duplicate_thread = sqlx::query("insert into dm_threads default values returning id")
        .fetch_one(&pool)
        .await?;
    let duplicate_thread_id: Uuid = duplicate_thread.try_get("id")?;
    sqlx::query(
        "insert into dm_members (thread_id, user_id)
         values ($1, $2), ($1, $3)",
    )
    .bind(duplicate_thread_id)
    .bind(first)
    .bind(second)
    .execute(&pool)
    .await?;
    let duplicate_pair = sqlx::query(
        "insert into dm_direct_threads (thread_id, first_user_id, second_user_id)
         values ($1, $2, $3)",
    )
    .bind(duplicate_thread_id)
    .bind(first.min(second))
    .bind(first.max(second))
    .execute(&pool)
    .await;

    assert!(extra_member.is_err());
    assert!(malformed_direct_thread.is_err());
    assert!(duplicate_pair.is_err());
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn direct_message_blocks_are_enforced_in_both_directions(pool: PgPool) -> sqlx::Result<()> {
    let first = insert_user(&pool, "first").await?;
    let second = insert_user(&pool, "second").await?;
    let thread_id = insert_direct_thread(&pool, first, second).await?;

    sqlx::query("insert into blocks (blocker_id, blocked_id) values ($1, $2)")
        .bind(first)
        .bind(second)
        .execute(&pool)
        .await?;
    let reverse_send = sqlx::query(
        "insert into dm_messages (thread_id, user_id, body)
         values ($1, $2, 'blocked reverse send')",
    )
    .bind(thread_id)
    .bind(second)
    .execute(&pool)
    .await;

    sqlx::query("delete from blocks where blocker_id = $1 and blocked_id = $2")
        .bind(first)
        .bind(second)
        .execute(&pool)
        .await?;
    sqlx::query("insert into blocks (blocker_id, blocked_id) values ($1, $2)")
        .bind(second)
        .bind(first)
        .execute(&pool)
        .await?;
    let forward_send = sqlx::query(
        "insert into dm_messages (thread_id, user_id, body)
         values ($1, $2, 'blocked forward send')",
    )
    .bind(thread_id)
    .bind(first)
    .execute(&pool)
    .await;

    assert!(reverse_send.is_err());
    assert!(forward_send.is_err());
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn direct_thread_delivery_visibility_requires_membership_and_no_block(
    pool: PgPool,
) -> sqlx::Result<()> {
    let first = insert_user(&pool, "first").await?;
    let second = insert_user(&pool, "second").await?;
    let outsider = insert_user(&pool, "outsider").await?;
    let thread_id = insert_direct_thread(&pool, first, second).await?;

    assert!(direct_thread_visible_to(&pool, thread_id, first).await?);
    assert!(direct_thread_visible_to(&pool, thread_id, second).await?);
    assert!(!direct_thread_visible_to(&pool, thread_id, outsider).await?);

    sqlx::query("insert into blocks (blocker_id, blocked_id) values ($1, $2)")
        .bind(second)
        .bind(first)
        .execute(&pool)
        .await?;

    assert!(!direct_thread_visible_to(&pool, thread_id, first).await?);
    assert!(!direct_thread_visible_to(&pool, thread_id, second).await?);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn direct_messages_remain_report_compatible(pool: PgPool) -> sqlx::Result<()> {
    let author = insert_user(&pool, "author").await?;
    let reporter = insert_user(&pool, "reporter").await?;
    let thread_id = insert_direct_thread(&pool, author, reporter).await?;
    let row = sqlx::query(
        "insert into dm_messages (thread_id, user_id, body)
         values ($1, $2, 'reportable')
         returning id",
    )
    .bind(thread_id)
    .bind(author)
    .fetch_one(&pool)
    .await?;
    let message_id: Uuid = row.try_get("id")?;

    let reportable = sqlx::query_scalar::<_, bool>(
        "select exists(
             select 1
             from dm_messages m
             join dm_members member on member.thread_id = m.thread_id
             where m.id = $1
               and m.user_id = $2
               and member.user_id = $3
               and m.deleted_at is null
         )",
    )
    .bind(message_id)
    .bind(author)
    .bind(reporter)
    .fetch_one(&pool)
    .await?;
    sqlx::query(
        "insert into reports (
             reporter_id,
             reported_user_id,
             message_id,
             message_kind,
             reason
         ) values ($1, $2, $3, 'dm', 'spam')",
    )
    .bind(reporter)
    .bind(author)
    .bind(message_id)
    .execute(&pool)
    .await?;

    assert!(reportable);
    Ok(())
}

async fn insert_direct_thread(pool: &PgPool, first: Uuid, second: Uuid) -> sqlx::Result<Uuid> {
    let row = sqlx::query("insert into dm_threads default values returning id")
        .fetch_one(pool)
        .await?;
    let thread_id: Uuid = row.try_get("id")?;
    sqlx::query(
        "insert into dm_members (thread_id, user_id)
         values ($1, $2), ($1, $3)",
    )
    .bind(thread_id)
    .bind(first)
    .bind(second)
    .execute(pool)
    .await?;
    sqlx::query(
        "insert into dm_direct_threads (thread_id, first_user_id, second_user_id)
         values ($1, $2, $3)",
    )
    .bind(thread_id)
    .bind(first.min(second))
    .bind(first.max(second))
    .execute(pool)
    .await?;
    Ok(thread_id)
}

async fn direct_thread_visible_to(
    pool: &PgPool,
    thread_id: Uuid,
    viewer_id: Uuid,
) -> sqlx::Result<bool> {
    sqlx::query_scalar(
        "select exists(
             select 1
             from dm_direct_threads d
             join dm_members viewer_member
               on viewer_member.thread_id = d.thread_id and viewer_member.user_id = $2
             join users other on other.id = case
                 when d.first_user_id = $2 then d.second_user_id
                 else d.first_user_id
             end
             where d.thread_id = $1
               and (d.first_user_id = $2 or d.second_user_id = $2)
               and not exists (
                   select 1
                   from blocks b
                   where (b.blocker_id = $2 and b.blocked_id = other.id)
                      or (b.blocker_id = other.id and b.blocked_id = $2)
               )
         )",
    )
    .bind(thread_id)
    .bind(viewer_id)
    .fetch_one(pool)
    .await
}

async fn insert_user(pool: &PgPool, name: &str) -> sqlx::Result<Uuid> {
    let row = sqlx::query("insert into users (display_name) values ($1) returning id")
        .bind(name)
        .fetch_one(pool)
        .await?;
    row.try_get("id")
}
