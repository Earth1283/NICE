# NICE/1

**Networked Inter-LAN Clipboard Exchange**  
*A small LAN protocol for clipboard transfer, file transfer, and regrettably, chat.*

## 1. Goals

NICE is intended for trusted or semi-trusted local networks.

NICE provides:

- LAN peer discovery
- clipboard transfer
- file transfer
- LAN chat
- explicit user approval for incoming clipboard/file offers
- stable cryptographic device identity
- optional encrypted transport
- intentionally excellent Wireshark visibility
- questionable naming, sane semantics

NICE is not intended for Internet-facing deployment.

---

## 2. Discovery

Peers SHOULD advertise themselves using mDNS/DNS-SD.

Suggested service:

```text
_nice._tcp.local
```

Suggested default TCP port:

```text
6969
```

Discovery metadata MAY include:

```text
version=1
device=<human-readable device name>
secure=yes
```

Discovery MUST NOT imply trust.

---

## 3. Device Identity

Every installation generates a long-lived Ed25519 keypair.

The private key MUST remain local.

The Ed25519 public key is the stable device identity.

A device's current LAN IP is its canonical **chat display name**, because apparently we have standards.

Example:

```text
192.168.1.42
```

The IP MUST NOT be used as the cryptographic identity because DHCP exists.

Internally:

```text
identity = Ed25519 public key
display identity = current source IP
```

---

## 4. Transport

NICE/1 runs over TCP.

Two transport modes exist:

```text
SECURE
PLAINTEXT
```

### 4.1 SECURE

SECURE SHOULD be the default.

TLS 1.3 SHOULD be used.

Clients SHOULD authenticate using their persistent Ed25519 identities.

Previously paired clients MUST verify that the presented identity matches the stored identity.

An unexpected identity change MUST NOT silently succeed.

### 4.2 PLAINTEXT

PLAINTEXT exists for:

- debugging
- protocol development
- packet captures
- making Wireshark happy

It MUST require explicit user configuration.

Implementations SHOULD display a warning approximately equivalent to:

```text
WARNING: YOU ASKED WIRESHARK TO READ YOUR SHITPOSTS
```

Secure implementations MAY support explicit TLS session-key logging for Wireshark so that developers can inspect encrypted NICE traffic without disabling real encryption.

---

## 5. Frame Format

Every NICE frame begins with:

```text
+--------+---------+---------+----------+------------+---------+
| Magic  | Version | Opcode  | Reserved | Stream ID  | Length  |
| 1 byte | 1 byte  | 1 byte  | 1 byte   | 4 bytes    | 4 bytes |
+--------+---------+---------+----------+------------+---------+
| Payload...
+-------------------------------------------------------------+
```

All multibyte integers are unsigned big-endian.

### Magic

```text
0x69
```

Rationale:

```text
nice
```

### Version

NICE/1:

```text
0x01
```

### Reserved

MUST be zero in NICE/1.

Receiving nonzero reserved bits MAY produce `CPP`.

### Stream ID

Identifies an independent operation.

This permits, for example:

- a file transfer
- a clipboard offer
- an LKML conversation

to coexist on one connection.

Stream ID `0` is reserved for connection-level messages.

### Length

Payload length in bytes.

Implementations MUST impose reasonable allocation limits before reading payload data.

---

## 6. Payload Encoding

Control-message payloads SHOULD use CBOR.

`DIFF` payloads contain binary transfer data directly, prefixed by the chunk offset.

Unknown CBOR fields SHOULD be ignored unless explicitly marked critical by a future protocol revision.

A client MUST NOT break existing peers merely because a new optional field exists.

See `BROKE_USERSPACE`.

---

# 7. Opcode Registry

```text
0x01  HELLO
0x02  MERGED

0x03  TUX
0x04  SUBSURFACE

0x10  PULL_REQUEST
0x11  MERGE
0x12  FUCK_OFF
0x13  SHUT_UP

0x20  DIFF
0x21  DONE
0x22  FSCK
0x23  CLEAN
0x24  CORRUPT
0x25  BIG_DIFF

0x30  LKML

0x40  BITKEEPER
0x41  GIT
0x42  MONOTONE

0x7C  CPP
0x7D  BROKE_USERSPACE
0x7E  NVIDIA
```

Additional opcodes MAY be defined in future revisions.

Existing opcode semantics MUST NOT be silently changed.

---

# 8. Connection Establishment

The initiating client sends:

```text
HELLO
```

`HELLO` contains at minimum:

```text
protocol version
device name
capabilities
transport mode
identity information
```

A conforming receiver accepting the connection responds:

```text
MERGED
```

Example:

```text
A -> B  HELLO
B -> A  MERGED
```

No application-level streams SHOULD begin before `MERGED`.

---

# 9. Keepalive

Either peer MAY transmit:

```text
TUX
```

after an implementation-defined period of inactivity.

A peer receiving `TUX` SHOULD respond:

```text
SUBSURFACE
```

Example:

```text
TX TUX
RX SUBSURFACE
```

Receipt of either message MUST NOT alter active transfer state.

Repeated failure to receive `SUBSURFACE` MAY cause the peer to be considered unreachable.

---

# 10. Clipboard and File Offers

Clipboard and files use the same offer mechanism.

The sender creates a stream and sends:

```text
PULL_REQUEST
```

Example file offer:

```text
PULL_REQUEST {
    kind: "file",
    name: "extremely_important_cat.png",
    mime: "image/png",
    size: 812944
}
```

Example clipboard offer:

```text
PULL_REQUEST {
    kind: "clipboard",
    mime: "text/plain",
    size: 482,
    preview: "look at this..."
}
```

Incoming clipboard and file requests SHOULD require explicit user approval by default.

The receiver responds with one of:

```text
MERGE
FUCK_OFF
SHUT_UP
BIG_DIFF
```

Semantics:

```text
MERGE
    Offer accepted.

FUCK_OFF
    Offer explicitly rejected.

SHUT_UP
    Sender is rate-limited. Payload SHOULD contain retry_after_ms.

BIG_DIFF
    Object exceeds a receiver preference or normal size threshold.
```

A sender MUST NOT begin transmitting `DIFF` before receiving `MERGE`.

Doing so constitutes `BROKE_USERSPACE`.

---

# 11. Data Transfer

Following `MERGE`, payload contents are transmitted using one or more:

```text
DIFF
```

frames.

A `DIFF` contains:

```text
offset
data
```

The sender concludes the object with:

```text
DONE
```

Example:

```text
PULL_REQUEST
MERGE
DIFF
DIFF
DIFF
DONE
```

---

# 12. Integrity Checking

Following `DONE`, the sender sends:

```text
FSCK
```

The payload contains:

```text
algorithm
digest
total_size
```

BLAKE3 SHOULD be the default digest algorithm for NICE/1 object transfers.

The receiver independently computes the digest and responds:

```text
CLEAN
```

if verification succeeds, or:

```text
CORRUPT
```

if it does not.

Happy path:

```text
DONE
FSCK
CLEAN
```

Sad path:

```text
DONE
FSCK
CORRUPT
```

---

# 13. LKML

NICE regrettably includes chat.

Chat traffic uses:

```text
LKML
```

An `LKML` payload contains at minimum:

```text
message_id
timestamp
text
```

The apparent username is the sender's current LAN IP.

Example:

```text
LKML

[192.168.1.14] this meme sucks
[192.168.1.27] merged
[192.168.1.14] fuck you
```

Stable identity remains the paired Ed25519 key.

Files sent through chat MUST reuse `PULL_REQUEST`; LKML MUST NOT implement an independent attachment protocol.

Implementations MAY display familiar kernel-style metadata such as:

```text
Acked-by:
Reviewed-by:
Tested-by:
```

because there is apparently no remaining adult supervision.

---

# 14. Rate Limiting

A peer sending excessive offers, chat messages, or other requests SHOULD receive:

```text
SHUT_UP
```

`SHUT_UP` SHOULD carry:

```text
retry_after_ms
```

Upon receiving it, a peer MUST cease initiating affected operations until the specified interval expires.

Example:

```text
RX PULL_REQUEST
RX PULL_REQUEST
RX PULL_REQUEST
RX PULL_REQUEST

TX SHUT_UP retry_after_ms=5000
```

Repeatedly ignoring `SHUT_UP` MAY result in `FUCK_OFF`, `NVIDIA`, or connection termination.

---

# 15. Session Recovery

If peers lose agreement about current session state, either peer MAY transmit:

```text
BITKEEPER
```

A conforming peer MUST respond:

```text
GIT
```

`GIT` establishes a fresh synchronization point.

Incomplete streams SHOULD be discarded or explicitly restarted.

Example:

```text
A -> B  BITKEEPER
B -> A  GIT
```

If a peer cannot or will not perform the specified recovery, it MAY reply:

```text
MONOTONE
```

`MONOTONE` therefore means approximately:

> I understood the problem, chose a technically defensible alternative, and have nevertheless disappointed the maintainer.

---

# 16. Error Semantics

## `CPP`

The received frame is syntactically grotesque, unsupported, unnecessarily complicated, or otherwise impossible to process correctly.

Typical causes:

- malformed payload
- impossible framing
- unsupported critical extension
- violation of basic encoding requirements

## `BROKE_USERSPACE`

The peer violated a compatibility or protocol-state guarantee.

Examples:

```text
DIFF before MERGE
changing established opcode semantics
unexpected stream-state transitions
breaking behavior relied upon by older clients
```

This is a serious protocol error.

## `NVIDIA`

The peer is refusing to cooperate at the connection level.

It SHOULD be reserved for fatal or persistent failures rather than ordinary user rejection.

The connection MAY be terminated immediately afterward.

## `MONOTONE`

The peer selected a valid-looking but non-conforming or deeply questionable alternative.

## `FUCK_OFF`

The requested operation was rejected.

This is not necessarily an error.

## `SHUT_UP`

Rate limit exceeded.

---

# 17. Security Model

A discovered device is not automatically trusted.

A first-time peer SHOULD require user confirmation before being paired.

Pairing stores the peer's Ed25519 identity.

Subsequent secure connections SHOULD authenticate that identity.

File and clipboard acceptance remain separate from device authentication.

Therefore:

```text
paired != permission to dump files onto my machine
```

Implementations SHOULD protect against:

- oversized allocations
- malicious filenames
- path traversal
- unsolicited file writes
- decompression bombs, if compression is later added
- chat flooding
- excessive connection attempts
- corrupted transfer state

Received files MUST NOT be written to arbitrary sender-selected paths.

---

# 18. Wireshark Considerations

A NICE implementation SHOULD be pleasant to dissect.

A NICE Wireshark dissector SHOULD display symbolic opcode names.

Example:

```text
NICE/1
    Magic: 0x69
    Version: 1
    Stream: 8
    Opcode: PULL_REQUEST
    Kind: file
    Name: cat.png
    Size: 812944

NICE/1
    Opcode: MERGE

NICE/1
    Opcode: DIFF

NICE/1
    Opcode: FSCK
    Algorithm: BLAKE3

NICE/1
    Opcode: CLEAN
```

Implementations using TLS SHOULD optionally support explicit developer-controlled key logging compatible with Wireshark.

---

# 19. Custom Client Warning

If you plan to implement a custom client, be prepared to receive a significant number of `BROKE_USERSPACE`, `CPP`, and `NVIDIA` responses.

This may be caused by:

- your questionable protocol interpretation,
- my questionable protocol design,
- undocumented behavior,
- documented behavior nobody expected you to actually implement,
- or a client simply deciding that what you have done is morally incorrect.

Implementations SHOULD attempt to distinguish between these cases.

Good luck.

---

# 20. Example Complete Transfer

```text
A -> B  HELLO
B -> A  MERGED

A -> B  TUX
B -> A  SUBSURFACE

A -> B  PULL_REQUEST
        kind=file
        name=shitpost.png
        size=42069

B -> A  MERGE

A -> B  DIFF offset=0
A -> B  DIFF offset=16384
A -> B  DIFF offset=32768

A -> B  DONE
A -> B  FSCK BLAKE3=<digest>

B -> A  CLEAN
```

Rejected clipboard:

```text
A -> B  PULL_REQUEST kind=clipboard
B -> A  FUCK_OFF
```

Rate limiting:

```text
A -> B  LKML "hello"
A -> B  LKML "hello"
A -> B  LKML "hello"
A -> B  LKML "hello"

B -> A  SHUT_UP retry_after_ms=10000
```

Recovery:

```text
A -> B  BITKEEPER
B -> A  GIT
```

Recovery, but somebody made a questionable architectural decision:

```text
A -> B  BITKEEPER
B -> A  MONOTONE
```

Protocol violation:

```text
A -> B  PULL_REQUEST
A -> B  DIFF

B -> A  BROKE_USERSPACE
B closes connection
```

---

# 21. Security Considerations

Yes.