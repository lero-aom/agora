-- The preceding migration inferred local-test sessions from a user's identities.
-- That cannot distinguish an older Steam session for a user who also has a dev identity.
alter table sessions
    drop constraint if exists sessions_auth_source_check;

with auth_source_migration as (
    select installed_on
    from _sqlx_migrations
    where version = 20260907009000
      and success
)
update sessions s
set auth_source = 'legacy',
    revoked_at = coalesce(s.revoked_at, now())
from auth_source_migration m
where s.auth_source = 'local_test'
  and s.created_at <= m.installed_on;

alter table sessions
    add constraint sessions_auth_source_check
    check (auth_source in ('steam', 'local_test', 'legacy')),
    add constraint sessions_legacy_source_revoked_check
    check (auth_source <> 'legacy' or revoked_at is not null);
