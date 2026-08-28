# Writing a client

A client drives the daemon over the control socket. This document covers how to build one;
[control-protocol.md](control-protocol.md) is the field-by-field reference.

There are two ways in. Most clients use the socket. A Rust client can skip it and embed
the library instead.

## Connecting

```
unix      $XDG_RUNTIME_DIR/nicer/nicer.sock, or the data directory if that is unset
windows   \\.\pipe\nicer
```

The daemon accepts `--socket` to put it elsewhere, which is how several daemons run on one
machine. A client should let the user override the path for the same reason.

Read the stream as lines. Write one JSON object per line. Nothing else is framing: no
length prefix, no delimiter but `\n`, and no requirement that a reply follow the command
that caused it before other lines arrive.

The socket is not authenticated beyond filesystem permissions, so a client holds the same
authority as the user running the daemon.

## The shape of a client

A client is a loop over incoming lines and a table of outstanding requests.

```
for line in socket:
    message = parse(line)
    if "reply" in message:
        resolve(outstanding.pop(message.get("id")), message)
    else:
        apply(message)          # an event
```

Correlate replies by `id`. The daemon echoes whatever JSON value the command carried, so
integers, strings, or UUIDs all work. A parse failure produces a reply with no `id`,
because the `id` could not be read; a client should surface those rather than discard
them.

Do not assume a reply arrives before the events its command caused. `connect` reliably
emits `connected` before it replies, because the reply is produced by the task that
finished the handshake.

## State a client should keep

The daemon is the source of truth. A client mirrors four things:

| State | Seeded by | Kept current by |
|---|---|---|
| This device | `status` | never changes while running |
| Discovered peers | — | `peer_discovered`, `peer_lost` |
| Paired peers | `list_peers` | after each `pair`, `unpair`, `auto_accept` |
| Connections | `list_connections` | `connected`, `disconnected` |
| Offers and transfers | — | `offer`, `offer_resolved`, `transfer_*` |

Offers and transfers are not enumerable. They exist only as events, so a client that
attaches mid-transfer will not learn about it. This is deliberate: an offer is a question
put to a user at a moment, not a resource with a lifetime a late observer can join.

Key transfers on `(connection, stream)`. A stream identifier is unique only within its
connection, and is reused after a `resynced`.

## Order of operations at startup

Subscribe before you ask. Events begin the moment the connection opens, and a client that
issues `status` first can miss an `offer` that arrives while it waits.

1. Connect. Start reading lines immediately, queueing events.
2. Send `status`, `list_peers`, `list_connections`.
3. Apply the replies as the base state, then apply the queued events on top.

Events applied twice must be harmless. They are: every event is a statement about a
`(connection, stream)` or a fingerprint, not a delta.

## Reconnecting

If the socket closes, the daemon may or may not still be running. On reconnect, discard
mirrored state and redo the startup sequence. Do not attempt to resume: connection
identifiers are assigned per daemon run and are not stable across a restart.

A client that falls behind on events is told so and then keeps running:

```json
{"event":"warning","message":"128 events were dropped; this client is behind"}
```

Treat that as a signal to re-read `list_connections` and `list_peers`. Transfers that
completed during the gap are unrecoverable from the event stream; the file is on disk
either way.

## The flows worth getting right

**Pairing.** A `pairing_required` event is a question for a human. Show `short` — the
grouped eight-octet form, `9f3a1c04 b7e2d580` — and make the full 64-character value
reachable. The user is comparing it against the other device's screen, which is the only
thing that makes trust on first use mean anything. Do not offer to pair from a
`peer_discovered` event: nothing there is authenticated.

**Identity changes.** `identity_changed` is not a pairing prompt and must not be rendered
as one. It means a stored identity no longer matches, and the connection already failed.
The only resolutions are `unpair` or investigating why. A client that offers "trust this
new key?" here has removed the point of pairing.

**Offers.** An `offer` event with `auto_accepted: false` needs a verdict, or the daemon
refuses it after five minutes. Show `kind`, `size`, and `name` or `preview`. A client that
wants a size threshold of its own should answer `big_diff` with `max_size` rather than
`fuck_off`, so the sender learns what would have fit.

**Auto-acceptance.** `auto_accept` requires the peer to be paired. Present it as a per-peer,
per-kind setting, because that is what it is. It is the only way an object lands without a
prompt, so it deserves to be visible and easy to revoke.

**Sending.** `send_file` takes a path on the daemon's filesystem, not the client's. A
client running on another machine cannot send a local file by path; it has no route to do
so, and should not present one.

## Rendering remote input

`device`, `name`, `preview`, `text`, and `message_id` originate with a peer. The daemon
strips control characters, truncates to documented limits, and reduces `name` to a bare
filename with no path separators. It does not know what will render the result.

A client is still responsible for escaping according to its own medium, for not
interpreting a name as a path, and for not letting a device name impersonate interface
text. Chat text is arbitrary UTF-8 up to 4096 octets.

The address in a `chat` event is the display name the protocol asks for. It is not an
identity. Show the fingerprint wherever identity matters, and be prepared for two peers to
share an address over time.

## A minimal client

Python, complete enough to accept a file:

```python
import json, socket, threading, queue, itertools

class Nicer:
    def __init__(self, path):
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.connect(path)
        self.io = self.sock.makefile("rwb")
        self.ids = itertools.count(1)
        self.pending, self.events = {}, queue.Queue()
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        for line in self.io:
            message = json.loads(line)
            if "reply" in message:
                self.pending.pop(message.get("id"), queue.Queue()).put(message)
            else:
                self.events.put(message)

    def call(self, **command):
        command["id"] = next(self.ids)
        answer = self.pending.setdefault(command["id"], queue.Queue())
        self.io.write((json.dumps(command) + "\n").encode())
        self.io.flush()
        return answer.get(timeout=30)

nicer = Nicer("/run/user/1000/nicer/nicer.sock")
print(nicer.call(cmd="status"))

while True:
    event = nicer.events.get()
    if event["event"] == "pairing_required":
        print("pair with", event["device"], event["short"], "?")
    elif event["event"] == "offer" and not event["auto_accepted"]:
        print("accept", event.get("name"), event["size"], "bytes?")
        nicer.call(cmd="respond", connection=event["connection"],
                   stream=event["stream"], verdict="merge")
    elif event["event"] == "transfer_complete":
        print("landed at", event.get("path"))
```

TypeScript, the same idea over Node's `net`:

```ts
import { createConnection } from "node:net";
import { createInterface } from "node:readline";

const socket = createConnection("/run/user/1000/nicer/nicer.sock");
const pending = new Map<number, (reply: any) => void>();
let nextId = 1;

createInterface({ input: socket }).on("line", (line) => {
  const message = JSON.parse(line);
  if ("reply" in message) pending.get(message.id)?.(message);
  else handleEvent(message);
});

function call(command: Record<string, unknown>): Promise<any> {
  const id = nextId++;
  return new Promise((resolve) => {
    pending.set(id, resolve);
    socket.write(JSON.stringify({ id, ...command }) + "\n");
  });
}
```

## Embedding instead

A Rust client can skip the socket and use the library. `Node::build` returns the node, a
handle, and its inbox; the handle carries the same `Command` and `Event` types the NDJSON
interface serialises.

```rust
use nicer::clipboard;
use nicer::config::Config;
use nicer::event::{Command, Event};
use nicer::node::{Node, Reply};
use nicer::paths::Paths;

let paths = Paths::discover()?;
let config = Config::load(&paths)?;
let (node, handle, inbox) = Node::build(config, &paths, clipboard::system_or_memory())?;

let mut events = handle.subscribe();
tokio::spawn(async move { node.run(inbox).await });

while let Ok(event) = events.recv().await {
    if let Event::Offer { connection, stream, .. } = event {
        handle.call(Command::Respond {
            connection,
            stream,
            verdict: nicer::event::Verdict::Merge,
        }).await;
    }
}
```

This gives typed events, no serialisation, and a `Clipboard` implementation of your own —
useful when the frontend owns the display server session and the daemon does not. The
cost is that the frontend becomes the daemon: one process, one lifetime, Rust only.

Use the socket if the interface is a separate program, might be written in something else,
or should survive the frontend restarting. Embed if you are shipping one Rust application.
