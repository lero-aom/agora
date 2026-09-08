create extension if not exists pgcrypto;

create table users (
    id uuid primary key default gen_random_uuid(),
    display_name text not null,
    avatar_url text null,
    role text not null default 'user' check (role in ('user', 'moderator', 'admin')),
    created_at timestamptz not null default now(),
    last_seen_at timestamptz null,
    suspended_until timestamptz null,
    banned_at timestamptz null
);

create table identities (
    id uuid primary key default gen_random_uuid(),
    user_id uuid not null references users(id) on delete cascade,
    provider text not null,
    provider_user_id text not null,
    provider_display_name text null,
    provider_avatar_url text null,
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now(),
    unique (provider, provider_user_id)
);

create table sessions (
    id uuid primary key default gen_random_uuid(),
    user_id uuid not null references users(id) on delete cascade,
    refresh_token_hash text not null,
    created_at timestamptz not null default now(),
    expires_at timestamptz not null,
    revoked_at timestamptz null,
    user_agent text null,
    last_used_at timestamptz null
);

create index sessions_user_id_idx on sessions(user_id);
create index sessions_refresh_token_hash_idx on sessions(refresh_token_hash);

create table global_messages (
    id uuid primary key default gen_random_uuid(),
    user_id uuid not null references users(id),
    body text not null check (char_length(body) between 1 and 1000),
    created_at timestamptz not null default now(),
    deleted_at timestamptz null,
    deleted_by uuid null references users(id)
);

create index global_messages_created_at_idx on global_messages(created_at desc);

create table dm_threads (
    id uuid primary key default gen_random_uuid(),
    created_at timestamptz not null default now()
);

create table dm_members (
    thread_id uuid not null references dm_threads(id) on delete cascade,
    user_id uuid not null references users(id) on delete cascade,
    last_read_message_id uuid null,
    primary key (thread_id, user_id)
);

create table dm_messages (
    id uuid primary key default gen_random_uuid(),
    thread_id uuid not null references dm_threads(id) on delete cascade,
    user_id uuid not null references users(id),
    body text not null check (char_length(body) between 1 and 1000),
    created_at timestamptz not null default now(),
    deleted_at timestamptz null
);

create index dm_messages_thread_created_at_idx on dm_messages(thread_id, created_at desc);

create table friendships (
    id uuid primary key default gen_random_uuid(),
    requester_id uuid not null references users(id) on delete cascade,
    addressee_id uuid not null references users(id) on delete cascade,
    status text not null check (status in ('pending', 'accepted', 'declined', 'removed')),
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now(),
    check (requester_id <> addressee_id)
);

create unique index friendships_pair_unique_idx on friendships (
    least(requester_id, addressee_id),
    greatest(requester_id, addressee_id)
);

create table blocks (
    blocker_id uuid not null references users(id) on delete cascade,
    blocked_id uuid not null references users(id) on delete cascade,
    created_at timestamptz not null default now(),
    primary key (blocker_id, blocked_id),
    check (blocker_id <> blocked_id)
);

create table reports (
    id uuid primary key default gen_random_uuid(),
    reporter_id uuid not null references users(id),
    reported_user_id uuid not null references users(id),
    message_id uuid null,
    message_kind text null check (message_kind is null or message_kind in ('global', 'dm')),
    reason text not null,
    details text null,
    status text not null default 'open' check (status in ('open', 'resolved', 'dismissed')),
    created_at timestamptz not null default now(),
    resolved_at timestamptz null,
    resolved_by uuid null references users(id)
);

create index reports_status_created_at_idx on reports(status, created_at desc);

create table moderation_actions (
    id uuid primary key default gen_random_uuid(),
    moderator_id uuid not null references users(id),
    target_user_id uuid not null references users(id),
    action text not null check (action in ('delete_message', 'timeout', 'suspend', 'ban')),
    reason text not null,
    created_at timestamptz not null default now(),
    expires_at timestamptz null
);

create index moderation_actions_target_created_at_idx on moderation_actions(target_user_id, created_at desc);
