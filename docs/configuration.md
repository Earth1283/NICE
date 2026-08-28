# Configuration

## Files

`nicer` uses platform conventions. The `directories` crate resolves them.

| | Linux | macOS | Windows |
|---|---|---|---|
| `config.toml` | `~/.config/nicer/` | `~/Library/Application Support/nicer/` | `%APPDATA%\nicer\config\` |
| `identity.key`, `peers.json` | `~/.local/share/nicer/` | `~/Library/Application Support/nicer/` | `%APPDATA%\nicer\data\` |
| Control endpoint | `$XDG_RUNTIME_DIR/nicer/nicer.sock` | data directory | `\\.\pipe\nicer` |

When `XDG_RUNTIME_DIR` is unset, the socket falls back to the data directory.

`nicer config` prints the effective configuration and the path it was read from. A missing
`config.toml` is not an error; defaults apply.

## Command line

```
nicer [--root DIR] [--config-dir DIR] [SUBCOMMAND]

  run        run the daemon (the default)
  identity   print this device's fingerprint and exit
  config     print the effective configuration and exit
```

`--root DIR` puts config, data, and the runtime directory under one tree, which is how
several daemons share a machine. `--config-dir DIR` moves only the configuration.

`run` accepts:

| Flag | Effect |
|---|---|
| `--listen ADDR` | Override `listen` |
| `--device NAME` | Override `device_name` |
| `--socket PATH` | Override the control endpoint |
| `--no-discovery` | Turn mDNS off |
| `--plaintext` | Ask for PLAINTEXT. Not sufficient on its own |

Flags override the file. `--plaintext` is deliberately not sufficient: the file must also
set `transport.i_know_wireshark_can_read_this`, so PLAINTEXT is never one flag away.

`NICER_LOG` sets the tracing filter, for example `NICER_LOG=nicer=debug`.

## `config.toml`

```toml
device_name = "thinkpad"
listen = "0.0.0.0:6969"
max_frame_size = 1048576
download_dir = "/home/you/Downloads/nicer"
discovery = true
# keylog = "/tmp/nicer-keys.log"

[transport]
mode = "secure"
i_know_wireshark_can_read_this = false

[limits]
max_streams_per_connection = 64
max_pending_offers = 8
max_connections_per_address = 4
max_object_size = 8589934592
offers_per_minute = 30
chat_per_minute = 120
frames_per_second = 512
chunk_size = 65536

[timeouts]
keepalive_secs = 30
idle_secs = 90
handshake_secs = 10
frame_secs = 30
```

### Top level

| Key | Default | Meaning |
|---|---|---|
| `device_name` | the hostname | Shown to peers, advertised over mDNS, and placed in the certificate's Common Name. Advisory; never an identity |
| `listen` | `0.0.0.0:6969` | Where NICE/1 peers connect |
| `max_frame_size` | 1048576 | The largest payload this daemon will accept, advertised in `HELLO`. At least 65536, at most 16777216 |
| `download_dir` | `~/Downloads/nicer` | Where received files land, and the only place they can |
| `discovery` | `true` | Advertise and browse over mDNS |
| `keylog` | unset | Write TLS session keys here for Wireshark. Off unless set, and warns when on |

`max_frame_size` is a negotiated ceiling, not a chunk size. Raising it lets peers send
larger frames, which costs memory per connection. The floor of 65536 is mandatory: every
NICE/1 implementation must accept that much, so a smaller value is not conformant.

### `[transport]`

| Key | Default | Meaning |
|---|---|---|
| `mode` | `"secure"` | `"secure"` or `"plaintext"` |
| `i_know_wireshark_can_read_this` | `false` | Must be `true` for PLAINTEXT to start |

A SECURE listener accepts only TLS and never falls back. A PLAINTEXT one proves no
identity: peers cannot be paired, existing pairings are not satisfied, and every peer is
reported as unauthenticated.

### `[limits]`

| Key | Default | Meaning |
|---|---|---|
| `max_streams_per_connection` | 64 | Open streams before offers are refused with `SHUT_UP` |
| `max_pending_offers` | 8 | Offers awaiting a verdict before further ones are refused |
| `max_connections_per_address` | 4 | Inbound connections per source address |
| `max_object_size` | 8 GiB | Larger offers are answered `BIG_DIFF` carrying this value |
| `offers_per_minute` | 30 | Offer budget per connection |
| `chat_per_minute` | 120 | Chat budget per connection |
| `frames_per_second` | 512 | Frame budget per connection |
| `chunk_size` | 65536 | Octets per outgoing `DIFF`, capped by the peer's `max_frame_size` |

The three budgets are leaky buckets: the configured rate is both the refill rate and the
burst capacity. Exceeding one produces `SHUT_UP` carrying how long to wait.

`chunk_size` must not exceed `max_frame_size`. Larger chunks mean fewer frames and more
memory in flight; the default matches the mandatory frame floor, so it is always sendable.

### `[timeouts]`

| Key | Default | Meaning |
|---|---|---|
| `keepalive_secs` | 30 | Inactivity before `TUX` is sent |
| `idle_secs` | 90 | Total silence before the connection closes |
| `handshake_secs` | 10 | TLS and `HELLO`/`MERGED` must complete within this |
| `frame_secs` | 30 | A frame whose header arrived must complete within this |

Three unanswered `TUX` also close a connection, whichever comes first.

## Validation

The daemon refuses to start rather than run a configuration it cannot honour. It rejects:

- an empty `device_name`
- `max_frame_size` below 65536 or above 16777216
- `chunk_size` of zero, or larger than `max_frame_size`
- `mode = "plaintext"` without `i_know_wireshark_can_read_this = true`

## Running two daemons on one machine

```sh
nicer --root /tmp/a run --listen 127.0.0.1:16969 --socket /tmp/a.sock --no-discovery
nicer --root /tmp/b run --listen 127.0.0.1:16970 --socket /tmp/b.sock --no-discovery
```

Each gets its own identity, peer store, and download directory. Discovery is off because
two instances on one host advertise into the same namespace.
