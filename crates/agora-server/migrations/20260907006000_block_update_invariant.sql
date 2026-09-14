drop trigger if exists blocks_remove_friendships on blocks;

create trigger blocks_remove_friendships
before insert or update of blocker_id, blocked_id on blocks
for each row
execute function remove_friendships_before_block();
