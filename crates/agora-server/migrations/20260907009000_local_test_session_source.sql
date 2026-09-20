alter table sessions
    add column auth_source text null;

update sessions s
set auth_source = case
    when exists (
        select 1
        from identities i
        where i.user_id = s.user_id
          and i.provider = 'dev'
    ) then 'local_test'
    else 'steam'
end
where auth_source is null;

alter table sessions
    alter column auth_source set not null;

alter table sessions
    add constraint sessions_auth_source_check
    check (auth_source in ('steam', 'local_test'));
