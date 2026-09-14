create or replace function reject_moderation_action_mutation()
returns trigger
language plpgsql
as $$
begin
    raise exception 'moderation_actions are immutable'
        using errcode = '55000';
end
$$;

create trigger moderation_actions_immutable
before update or delete on moderation_actions
for each row
execute function reject_moderation_action_mutation();

create or replace function notify_session_revoked()
returns trigger
language plpgsql
as $$
begin
    if old.revoked_at is null and new.revoked_at is not null then
        perform pg_notify('agora_session_revoked', new.id::text);
    end if;

    return new;
end
$$;

create trigger sessions_revoked_notify
after update of revoked_at on sessions
for each row
execute function notify_session_revoked();

create or replace function notify_user_restricted()
returns trigger
language plpgsql
as $$
begin
    if (old.banned_at is distinct from new.banned_at and new.banned_at is not null)
        or (
            old.suspended_until is distinct from new.suspended_until
            and new.suspended_until is not null
            and new.suspended_until > now()
        ) then
        perform pg_notify('agora_user_restricted', new.id::text);
    end if;

    return new;
end
$$;

create trigger users_restricted_notify
after update of banned_at, suspended_until on users
for each row
execute function notify_user_restricted();
