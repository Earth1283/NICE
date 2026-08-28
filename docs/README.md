# nicer documentation

`nicer` is a NICE/1 daemon. The protocol it speaks is defined by two documents at the
repository root:

- [`NICE-1.md`](../NICE-1.md) — the protocol.
- [`NICE-1-RFC.md`](../NICE-1-RFC.md) — what NICE/1 leaves open, resolved. Normative for
  this implementation and referenced throughout these documents as R1…R15.

These documents describe the daemon rather than the protocol:

| Document | Covers |
|---|---|
| [architecture.md](architecture.md) | Crate and module layout, the task and channel model, the session state machine |
| [behaviour.md](behaviour.md) | What the daemon does: discovery, connections, offers, transfers, chat, rate limiting, recovery |
| [control-protocol.md](control-protocol.md) | The NDJSON interface a frontend drives the daemon with |
| [client-guide.md](client-guide.md) | Writing a frontend against that interface, or embedding the library instead |
| [interop.md](interop.md) | Writing a NICE/1 peer that talks to `nicer` over the network |
| [configuration.md](configuration.md) | `config.toml`, command-line flags, file locations |
| [security.md](security.md) | Identity, pairing, authorisation, resource bounds, threat model |

Where to start:

- **Building a frontend** — [client-guide.md](client-guide.md), then
  [control-protocol.md](control-protocol.md) as reference, then
  [security.md](security.md) for the pairing and offer flows you must not get wrong.
- **Implementing NICE/1 elsewhere** — [interop.md](interop.md), then
  [`NICE-1-RFC.md`](../NICE-1-RFC.md).
- **Working on the daemon** — [architecture.md](architecture.md), then
  [behaviour.md](behaviour.md).
