alter table sessions
    add column access_token_hash text null,
    add column access_token_expires_at timestamptz null;

create index sessions_access_token_hash_idx on sessions(access_token_hash);

create table steam_login_challenges (
    id uuid primary key default gen_random_uuid(),
    poll_token_hash text not null unique,
    status text not null default 'pending' check (status in ('pending', 'complete', 'expired', 'denied')),
    user_id uuid null references users(id) on delete set null,
    error text null,
    created_at timestamptz not null default now(),
    expires_at timestamptz not null,
    completed_at timestamptz null,
    consumed_at timestamptz null
);

create index steam_login_challenges_status_expires_at_idx on steam_login_challenges(status, expires_at);
