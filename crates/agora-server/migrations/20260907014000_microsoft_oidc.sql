alter table sessions
    drop constraint sessions_auth_source_check;

alter table sessions
    add constraint sessions_auth_source_check
    check (auth_source in ('steam', 'microsoft', 'local_test', 'legacy'));

create table microsoft_login_challenges (
    id uuid primary key default gen_random_uuid(),
    poll_token_hash text not null unique,
    state_hash text not null unique,
    nonce_hash text not null,
    status text not null default 'pending' check (status in ('pending', 'complete', 'expired', 'denied')),
    user_id uuid null references users(id) on delete set null,
    error text null,
    created_at timestamptz not null default now(),
    expires_at timestamptz not null,
    callback_started_at timestamptz null,
    completed_at timestamptz null,
    consumed_at timestamptz null
);

create index microsoft_login_challenges_status_expires_at_idx
    on microsoft_login_challenges(status, expires_at);
