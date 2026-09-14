create or replace function relationship_pair_lock_key(first_user_id uuid, second_user_id uuid)
returns bigint
language sql
immutable
as $$
    select hashtextextended(
        least(first_user_id, second_user_id)::text || ':' || greatest(first_user_id, second_user_id)::text,
        0
    )
$$;

create or replace function lock_relationship_pair(first_user_id uuid, second_user_id uuid)
returns void
language sql
as $$
    select pg_advisory_xact_lock(relationship_pair_lock_key(first_user_id, second_user_id))
$$;

update friendships f
set status = 'removed', updated_at = now()
where status in ('pending', 'accepted')
  and exists (
      select 1
      from blocks b
      where (b.blocker_id = f.requester_id and b.blocked_id = f.addressee_id)
         or (b.blocker_id = f.addressee_id and b.blocked_id = f.requester_id)
  );

create or replace function enforce_friendship_block_invariant()
returns trigger
language plpgsql
as $$
begin
    perform lock_relationship_pair(new.requester_id, new.addressee_id);

    if new.status in ('pending', 'accepted') and exists (
        select 1
        from blocks b
        where (b.blocker_id = new.requester_id and b.blocked_id = new.addressee_id)
           or (b.blocker_id = new.addressee_id and b.blocked_id = new.requester_id)
    ) then
        raise exception 'active friendship cannot exist while either user has blocked the other'
            using errcode = '23514';
    end if;

    return new;
end
$$;

create trigger friendships_block_invariant
before insert or update of requester_id, addressee_id, status on friendships
for each row
execute function enforce_friendship_block_invariant();

create or replace function remove_friendships_before_block()
returns trigger
language plpgsql
as $$
begin
    perform lock_relationship_pair(new.blocker_id, new.blocked_id);

    update friendships
    set status = 'removed', updated_at = now()
    where status <> 'removed'
      and ((requester_id = new.blocker_id and addressee_id = new.blocked_id)
        or (requester_id = new.blocked_id and addressee_id = new.blocker_id));

    return new;
end
$$;

create trigger blocks_remove_friendships
before insert on blocks
for each row
execute function remove_friendships_before_block();
