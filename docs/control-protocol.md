# Control protocol

The daemon is driven over a Unix domain socket, or a named pipe on Windows. The transport
carries newline-delimited JSON: one object per line in both directions, UTF-8, no framing
beyond the newline.

Default endpoints:

```
unix      $XDG_RUNTIME_DIR/nicer/nicer.sock, or the data directory if that is unset
windows   \\.\pipe\nicer
```

On unix the socket is created with mode `0600`. Anyone who can open it can drive the
daemon completely: accept transfers, read the clipboard, unpair devices. It is not
authenticated beyond filesystem permissions.

## Message shapes

A line the client sends is a **command**. It carries `cmd`, the command's fields, and an
optional `id` of any JSON type.

A line the daemon sends is either a **reply** or an **event**:

- A reply has a `reply` field, and echoes the `id` of the command it answers.
- An event has an `event` field and no `id`.

Clients distinguish the two by which field is present. Every command produces exactly one
reply. Events arrive at any time, including between a command and its reply, and including
before any command has been sent.

Multiple clients may connect. Each receives every event.

A command that cannot be parsed produces an error reply with no `id`, since the `id` could
not be read.

```json
{"reply":"error","message":"unknown variant `nonsense`, expected one of `status`, …"}
```

## Commands

| `cmd` | Fields | Reply |
|---|---|---|
| `status` | — | `status` |
| `list_peers` | — | `peers` |
| `list_connections` | — | `connections` |
| `connect` | `address` | `connected` or `error` |
| `disconnect` | `connection` | `ok` or `error` |
| `pair` | `fingerprint`, optional `device` | `ok` or `error` |
| `unpair` | `fingerprint` | `ok` or `error` |
| `auto_accept` | `fingerprint`, `kind`, `enabled` | `ok` or `error` |
| `send_file` | `connection`, `path` | `ok` or `error` |
| `send_clipboard` | `connection`, optional `text` | `ok` or `error` |
| `respond` | `connection`, `stream`, `verdict`, verdict fields | `ok` or `error` |
| `chat` | `connection`, `text` | `ok` or `error` |
| `resync` | `connection` | `ok` or `error` |

Field types:

| Field | Type |
|---|---|
| `address` | `"host:port"`, an IP address and port |
| `connection` | integer, from a `connected` event or reply |
| `stream` | integer, from an `offer` event |
| `fingerprint` | 64 lowercase hex characters |
| `kind` | `"file"` or `"clipboard"` |
| `path` | an absolute path on the daemon's filesystem |

Notes:

- `send_clipboard` without `text` reads the daemon's own clipboard.
- `send_file` reads the path on the machine the daemon runs on, not the client's.
- `pair` without `device` uses the name from an open connection with that fingerprint, or
  the short fingerprint.
- `auto_accept` requires the peer to be paired already.
- An `ok` reply to `send_file`, `chat`, or `respond` means the instruction reached the
  session, not that the peer accepted anything. The outcome arrives as an event.

## Verdicts

`respond` carries a verdict inline:

```json
{"cmd":"respond","connection":1,"stream":8,"verdict":"merge"}
{"cmd":"respond","connection":1,"stream":8,"verdict":"fuck_off","reason":"this meme sucks"}
{"cmd":"respond","connection":1,"stream":8,"verdict":"big_diff","max_size":1048576}
```

`reason` and `max_size` are optional. The same three shapes appear inside `offer_resolved`.

## Replies

```json
{"id":1,"reply":"ok"}
{"id":1,"reply":"error","message":"connection 3 is not open"}
{"id":1,"reply":"connected","connection":1}
```

`status` flattens its fields into the reply object:

```json
{"id":1,"reply":"status","device":"alpha",
 "fingerprint":"9f3a…","short":"9f3a1c04 b7e2d580",
 "listen":"0.0.0.0:6969","transport":"secure","discovery":true,
 "download_dir":"/home/you/Downloads/nicer","connections":1,"paired_peers":2}
```

`peers` carries the stored peer records:

```json
{"id":1,"reply":"peers","peers":[
  {"fingerprint":"9f3a…","device":"thinkpad","paired_at":1787920235,
   "last_seen":1787920240,"last_address":"192.168.1.42",
   "auto_accept_files":false,"auto_accept_clipboard":true}]}
```

`connections` carries the open connections:

```json
{"id":1,"reply":"connections","connections":[
  {"connection":1,"address":"192.168.1.42:6969","device":"thinkpad",
   "transport":"secure","direction":"outgoing","fingerprint":"9f3a…","paired":true}]}
```

`direction` is `outgoing` if this daemon dialled, `incoming` if it accepted.

## Events

| `event` | Fields | Meaning |
|---|---|---|
| `listening` | `address`, `transport`, `device`, `fingerprint` | The daemon is up |
| `peer_discovered` | `instance`, `device`, `addresses`, `port`, `secure`, `fingerprint?` | An mDNS advertisement. Implies nothing |
| `peer_lost` | `instance` | An advertisement went away |
| `connected` | `connection`, `address`, `device`, `transport`, `direction`, `fingerprint?`, `paired` | A connection is usable |
| `disconnected` | `connection`, `reason` | It is not |
| `pairing_required` | `connection`, `address`, `device`, `fingerprint`, `short` | An authenticated but unpaired peer. Show `short`, offer to pair |
| `identity_changed` | `address`, `expected`, `reason` | A stored identity did not match. Not a pairing prompt |
| `offer` | `connection`, `stream`, `kind`, `size`, `name?`, `mime?`, `preview?`, `auto_accepted` | Someone wants to send something |
| `offer_resolved` | `connection`, `stream`, `direction`, `verdict`, verdict fields | An offer was answered |
| `transfer_progress` | `connection`, `stream`, `direction`, `transferred`, `total` | Emitted about every mebibyte |
| `transfer_complete` | `connection`, `stream`, `direction`, `kind`, `path?` | Verified and committed |
| `transfer_failed` | `connection`, `stream`, `direction`, `reason` | It did not arrive |
| `chat` | `connection`, `stream`, `from`, `fingerprint?`, `message_id`, `timestamp`, `text`, `tags?` | A message |
| `rate_limited` | `connection`, `direction`, `retry_after_ms`, `scope` | A `SHUT_UP` was sent or received |
| `protocol_error` | `connection`, `opcode`, `reason` | A `CPP`, `BROKE_USERSPACE`, or `NVIDIA` crossed the wire |
| `resynced` | `connection` | `BITKEEPER`/`GIT` completed; open streams were abandoned |
| `warning` | `message` | Something a user should know |

`direction` on a transfer or offer event is from this daemon's point of view: `incoming`
means it is arriving here.

`path` is present on `transfer_complete` only for a received file. A clipboard transfer
has already been applied; a sent file has no local destination.

`fingerprint` is absent when the connection is PLAINTEXT, because no identity was proven.

## A worked session

```json
→ {"id":1,"cmd":"status"}
← {"id":1,"reply":"status","device":"alpha","fingerprint":"3786…", …}

← {"event":"peer_discovered","instance":"thinkpad","device":"thinkpad",
   "addresses":["192.168.1.42"],"port":6969,"secure":true,"fingerprint":"9f3a…"}

→ {"id":2,"cmd":"connect","address":"192.168.1.42:6969"}
← {"event":"connected","connection":1,"address":"192.168.1.42:6969","device":"thinkpad",
   "transport":"secure","direction":"outgoing","fingerprint":"9f3a…","paired":false}
← {"event":"pairing_required","connection":1,"address":"192.168.1.42:6969",
   "device":"thinkpad","fingerprint":"9f3a…","short":"9f3a1c04 b7e2d580"}
← {"id":2,"reply":"connected","connection":1}

→ {"id":3,"cmd":"pair","fingerprint":"9f3a…"}
← {"id":3,"reply":"ok"}

← {"event":"offer","connection":1,"stream":2,"kind":"file",
   "name":"extremely_important_cat.png","mime":"image/png","size":812944,
   "auto_accepted":false}
→ {"id":4,"cmd":"respond","connection":1,"stream":2,"verdict":"merge"}
← {"id":4,"reply":"ok"}
← {"event":"transfer_progress","connection":1,"stream":2,"direction":"incoming",
   "transferred":1048576,"total":812944}
← {"event":"transfer_complete","connection":1,"stream":2,"direction":"incoming",
   "kind":"file","path":"/home/you/Downloads/nicer/extremely_important_cat.png"}

→ {"id":5,"cmd":"chat","connection":1,"text":"merged"}
← {"id":5,"reply":"ok"}
← {"event":"chat","connection":1,"stream":1,"from":"192.168.1.42","fingerprint":"9f3a…",
   "message_id":"1787920057315-1","timestamp":1787920057315,"text":"this meme sucks"}
```

## Notes for frontend authors

- Match replies by `id`. Do not assume ordering: a reply can be preceded by events caused
  by the same command.
- Treat `device`, `preview`, `text`, `name`, and `message_id` as untrusted remote input.
  The daemon strips control characters and truncates them, and reduces `name` to a bare
  filename, but it does not know what will render them.
- Show `short` for a fingerprint and make the full value reachable. Never identify a peer
  by address alone.
- `identity_changed` is not a pairing prompt. It means a stored identity no longer
  matches, and it is resolved by `unpair`, not by confirming.
- An event backlog that a client cannot keep up with is dropped, and the client is told:
  `{"event":"warning","message":"… events were dropped; this client is behind"}`. State
  can be re-read with `status`, `list_peers`, and `list_connections`.
