alter table sessions
    add column session_family_id uuid null,
    add column absolute_expires_at timestamptz null,
    add column refresh_token_used_at timestamptz null;

-- Existing sessions cannot safely receive a longer lifetime or share a family.
update sessions
set session_family_id = id,
    absolute_expires_at = expires_at
where session_family_id is null
   or absolute_expires_at is null;

alter table sessions
    alter column session_family_id set not null,
    alter column absolute_expires_at set not null;

-- A duplicated legacy hash cannot be attributed to one session family safely.
with duplicate_hashes as (
    select refresh_token_hash
    from sessions
    group by refresh_token_hash
    having count(*) > 1
)
update sessions s
set revoked_at = coalesce(s.revoked_at, now()),
    refresh_token_hash = s.refresh_token_hash || ':duplicate:' || s.id::text
from duplicate_hashes d
where s.refresh_token_hash = d.refresh_token_hash;

create unique index sessions_refresh_token_hash_unique_idx
    on sessions(refresh_token_hash);
create index sessions_session_family_id_idx
    on sessions(session_family_id);
create index sessions_absolute_expires_at_idx
    on sessions(absolute_expires_at);
create index sessions_revoked_at_idx
    on sessions(revoked_at)
    where revoked_at is not null;
create index steam_login_challenges_expires_at_idx
    on steam_login_challenges(expires_at);
