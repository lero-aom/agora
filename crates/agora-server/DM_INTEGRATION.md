# Direct Message Integration

Direct messages are registered in the server router, delivered through the authenticated WebSocket,
and surfaced in the desktop client.

## Router Registration

The REST endpoints are authenticated with the existing bearer-session extractor:

- `GET /dm/threads`
- `POST /dm/threads` with `CreateDmThreadRequest`
- `GET /dm/threads/{thread_id}/messages?before={message_id}&limit={1..100}`
- `POST /dm/threads/{thread_id}/messages` with `SendDmMessageRequest`

History is returned oldest-to-newest within each page. Use `next_before_message_id` as the next `before` cursor.

## Realtime Routing

Protocol version 6 adds the existing top-level `DmRealtimeEvent` wire shapes:

- `dm_thread_updated` carries a viewer-specific `DmThreadSummary`.
- `dm_message_created` carries the new `DmMessage`.

After a direct-thread creation or message transaction commits, the server broadcasts through a
DM-specific audience, never the unconditional user-event path. At delivery time it reloads the
viewer-specific thread summary, which rechecks canonical membership, active accounts, and the
bilateral block state. Missing membership, a block, or a delivery-query failure suppresses the
private event. This also prevents queued events from leaking after a block commits.

The client refreshes thread state after reconnect, broadcast recovery, and relationship changes;
it also deduplicates realtime messages with paginated REST history.
