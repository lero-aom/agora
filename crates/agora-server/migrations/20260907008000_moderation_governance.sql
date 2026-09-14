alter table users
    drop constraint if exists users_role_check;

alter table users
    add constraint users_role_check
    check (role in ('user', 'moderator', 'admin', 'owner'));

alter table moderation_actions
    drop constraint if exists moderation_actions_report_id_fkey;

alter table moderation_actions
    add constraint moderation_actions_report_id_fkey
    foreign key (report_id) references reports(id) on delete restrict;
