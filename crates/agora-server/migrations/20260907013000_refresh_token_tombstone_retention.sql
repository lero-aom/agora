create index sessions_refresh_token_used_at_idx
    on sessions(refresh_token_used_at)
    where refresh_token_used_at is not null;
