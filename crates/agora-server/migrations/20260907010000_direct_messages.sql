create table dm_direct_threads (
    thread_id uuid primary key references dm_threads(id) on delete cascade,
    first_user_id uuid not null,
    second_user_id uuid not null,
    constraint dm_direct_threads_canonical_pair_check check (first_user_id < second_user_id),
    constraint dm_direct_threads_pair_unique unique (first_user_id, second_user_id),
    constraint dm_direct_threads_first_member_fkey
        foreign key (thread_id, first_user_id)
        references dm_members(thread_id, user_id)
        on delete cascade,
    constraint dm_direct_threads_second_member_fkey
        foreign key (thread_id, second_user_id)
        references dm_members(thread_id, user_id)
        on delete cascade
);

create index dm_direct_threads_second_user_id_idx on dm_direct_threads(second_user_id);
create index dm_members_user_id_thread_id_idx on dm_members(user_id, thread_id);
create index dm_messages_visible_thread_created_at_id_idx
    on dm_messages(thread_id, created_at desc, id desc)
    where deleted_at is null;

create or replace function enforce_dm_direct_thread_shape()
returns trigger
language plpgsql
as $$
declare
    member_count bigint;
begin
    select count(*)
    into member_count
    from dm_members
    where thread_id = new.thread_id;

    if member_count <> 2
        or not exists (
            select 1
            from dm_members
            where thread_id = new.thread_id and user_id = new.first_user_id
        )
        or not exists (
            select 1
            from dm_members
            where thread_id = new.thread_id and user_id = new.second_user_id
        ) then
        raise exception 'direct message threads must have exactly their two canonical members'
            using errcode = '23514';
    end if;

    return new;
end
$$;

create trigger dm_direct_threads_shape_invariant
before insert or update of thread_id, first_user_id, second_user_id on dm_direct_threads
for each row
execute function enforce_dm_direct_thread_shape();

create or replace function reject_noncanonical_dm_direct_member()
returns trigger
language plpgsql
as $$
begin
    if exists (
        select 1
        from dm_direct_threads d
        where d.thread_id = new.thread_id
          and new.user_id <> d.first_user_id
          and new.user_id <> d.second_user_id
    ) then
        raise exception 'direct message threads only permit their two canonical members'
            using errcode = '23514';
    end if;

    return new;
end
$$;

create trigger dm_members_direct_thread_invariant
before insert or update of thread_id, user_id on dm_members
for each row
execute function reject_noncanonical_dm_direct_member();

create or replace function enforce_dm_direct_message_block_invariant()
returns trigger
language plpgsql
as $$
declare
    other_user_id uuid;
begin
    select case
        when d.first_user_id = new.user_id then d.second_user_id
        when d.second_user_id = new.user_id then d.first_user_id
        else null
    end
    into other_user_id
    from dm_direct_threads d
    where d.thread_id = new.thread_id;

    if found then
        if other_user_id is null then
            raise exception 'direct message author is not a thread member'
                using errcode = '23514';
        end if;

        perform lock_relationship_pair(new.user_id, other_user_id);

        if exists (
            select 1
            from blocks b
            where (b.blocker_id = new.user_id and b.blocked_id = other_user_id)
               or (b.blocker_id = other_user_id and b.blocked_id = new.user_id)
        ) then
            raise exception 'direct message cannot be sent while either user has blocked the other'
                using errcode = '23514';
        end if;
    end if;

    return new;
end
$$;

create trigger dm_messages_direct_thread_block_invariant
before insert or update of thread_id, user_id on dm_messages
for each row
execute function enforce_dm_direct_message_block_invariant();
