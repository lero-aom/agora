alter table dm_messages
    add constraint dm_messages_member_fk
    foreign key (thread_id, user_id) references dm_members(thread_id, user_id);

alter table reports
    add constraint reports_no_self_report_check check (reporter_id <> reported_user_id),
    add constraint reports_message_pair_check check (
        (message_id is null and message_kind is null)
        or (message_id is not null and message_kind is not null)
    );

create index identities_user_id_idx on identities(user_id);
create index friendships_requester_status_idx on friendships(requester_id, status);
create index friendships_addressee_status_idx on friendships(addressee_id, status);
create index blocks_blocked_id_idx on blocks(blocked_id);
