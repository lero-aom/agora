lock table reports in share row exclusive mode;

with ranked_open_reports as (
    select
        id,
        row_number() over (
            partition by
                reporter_id,
                reported_user_id,
                coalesce(message_id, '00000000-0000-0000-0000-000000000000'::uuid),
                coalesce(message_kind, '')
            order by created_at asc, id asc
        ) as duplicate_rank
    from reports
    where status = 'open'
)
update reports r
set
    status = 'dismissed',
    resolved_at = coalesce(resolved_at, now()),
    details = concat_ws(
        E'\n\n',
        nullif(r.details, ''),
        'Automatically dismissed during upgrade because it duplicated another open report.'
    )
from ranked_open_reports ranked
where r.id = ranked.id
  and ranked.duplicate_rank > 1;

create unique index reports_open_reporter_target_message_unique_idx on reports (
    reporter_id,
    reported_user_id,
    coalesce(message_id, '00000000-0000-0000-0000-000000000000'::uuid),
    coalesce(message_kind, '')
) where status = 'open';
