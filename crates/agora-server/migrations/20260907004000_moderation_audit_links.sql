alter table moderation_actions
    drop constraint moderation_actions_action_check;

update moderation_actions
set action = 'delete_global_message'
where action = 'delete_message';

update moderation_actions
set action = 'suspend'
where action = 'timeout';

alter table moderation_actions
    add column report_id uuid null references reports(id) on delete set null,
    add column message_id uuid null,
    add column message_kind text null check (message_kind is null or message_kind in ('global', 'dm')),
    add constraint moderation_actions_action_check check (
        action in (
            'delete_global_message',
            'suspend',
            'ban',
            'unban',
            'resolve_report',
            'dismiss_report'
        )
    ),
    add constraint moderation_actions_message_pair_check check (
        (message_id is null and message_kind is null)
        or (message_id is not null and message_kind is not null)
    );

create index moderation_actions_report_id_idx on moderation_actions(report_id);
create index moderation_actions_message_idx on moderation_actions(message_kind, message_id);
