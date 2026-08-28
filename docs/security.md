# Security model

NICE/1 is for trusted or semi-trusted local networks and is not meant to face the
internet (§1). What follows is what the daemon guarantees inside that scope, and what it
does not.

## Identity

Each installation holds one long-lived Ed25519 keypair, stored as PKCS#8 DER in
`identity.key` with owner-only permissions, created on first run. It never leaves the
machine.

The certificate that carries it into TLS is rebuilt at every start from the stored key, so
renaming the device changes nothing about its identity.

The **fingerprint** is `SHA-256(SubjectPublicKeyInfo DER)`, 64 lowercase hex characters.
Its short form groups the first eight octets:

```
9f3a1c04 b7e2d580
```

An address is not an identity. §3 makes the current LAN address the chat display name, and
the daemon reports both, but every trust decision is keyed on the fingerprint. DHCP means
an address is a suggestion.

## Transport authentication

SECURE is TLS 1.3 with mutual authentication. Both peers present exactly one self-signed
certificate whose `SubjectPublicKeyInfo` is their Ed25519 key. The server requests a client
certificate and refuses a connection without one; there are no anonymous clients.

Certificate validation is replaced rather than configured. There is no CA to chain to and
no meaningful hostname on a LAN, so the daemon checks that the chain holds one certificate,
that it is self-signed, and what its public key is. Everything else — issuer, name,
expiry — is ignored deliberately.

Possession of the private key is proven by the TLS `CertificateVerify` message. The
fingerprint in `HELLO` must match what the certificate proved; a mismatch fails the
connection.

Under PLAINTEXT nothing is proven. The fingerprint in `HELLO` is an unverified claim, and
the daemon treats it as one: a PLAINTEXT peer cannot be paired, does not satisfy an
existing pairing, and is reported to the frontend with no fingerprint at all.

## Pairing

Pairing is trust on first use, confirmed by a human. The daemon never pairs on its own and
never stores an identity without an explicit command.

Discovery does not imply trust. An mDNS advertisement — including its `fp` record — is
unauthenticated and trivially forged. It never triggers a connection or a pairing prompt.

When dialling a peer whose identity is stored for that address, that identity is pinned
and a mismatch fails the TLS handshake. The result is reported as `identity_changed`, not
as a new pairing prompt, and is cleared only by an explicit `unpair`. A frontend that
re-prompts here has removed the point of pairing.

## Authorisation is not authentication

Pairing authenticates a device. It does not authorise a transfer.

```
paired != permission to dump files onto my machine
```

Every offer requires explicit user approval. The single exception is a peer the user has
marked auto-accepting, which is per peer, per kind, never a default, and requires pairing
first. An offer nobody answers is refused after five minutes.

## Received objects

A sender does not choose where its file lands.

The name it supplies is reduced to a bare filename: everything before the last path
separator is dropped, along with anything before a drive-letter colon; control characters
and NUL are removed; leading and trailing dots and whitespace are stripped; the result is
truncated to 200 octets on a character boundary. A name that reduces to nothing becomes
`received`. `../../etc/passwd` becomes `passwd`.

The result is joined to the configured download directory and nowhere else. A name that
collides with an existing file is disambiguated — `cat.png`, then `cat (2).png` — and never
overwritten.

Every transfer is written to a temporary `.part` file and renamed into place only after
its digest verifies and the commit succeeds. A failed, refused, or corrupt transfer leaves
nothing that could be mistaken for a finished file. Abandoning a stream, including through
`BITKEEPER`/`GIT` recovery or a closing connection, removes the temporary file.

Clipboard content is held in memory and applied only after verification. It must be valid
UTF-8.

## Integrity

Every object is verified with BLAKE3 by default; SHA-256 is accepted. The receiver computes
the digest itself over what it actually received and compares in constant time.

Because the algorithm is named in `FSCK`, which arrives after the last `DIFF`, both
supported digests are computed as the object streams past.

A digest that does not match produces `CORRUPT` and the object is discarded. A digest that
matches but cannot be stored also produces `CORRUPT`, because from the sender's position it
did not arrive.

This is integrity against corruption and against a peer that lies about what it sent. It is
not authentication of content: a paired peer can send whatever it likes, correctly hashed.

## Resource bounds

| Bound | Default | Enforced by |
|---|---|---|
| Frame payload | 1 MiB | Refused from the header, before any buffer is reserved |
| Object size | 8 GiB | `BIG_DIFF` carrying the limit |
| Open streams | 64 per connection | `SHUT_UP` |
| Offers awaiting a verdict | 8 per connection | `SHUT_UP` |
| Inbound connections | 4 per source address | Refused before the handshake |
| Offers | 30 per minute | `SHUT_UP` |
| Chat messages | 120 per minute | `SHUT_UP` |
| Frames | 512 per second | `SHUT_UP`, then slowed reads |
| Chat text | 4096 octets | `CPP` |
| Preview | 256 octets | `CPP` |
| Device name | 63 characters | Truncated |
| Handshake | 10 seconds | Connection dropped |
| Idle connection | 90 seconds | Connection closed |
| Unanswered offer | 5 minutes | Refused automatically |

A peer that provokes more than five rate limits on one connection is sent `NVIDIA` and
disconnected.

Frames are never dropped to enforce a rate, because dropping one would corrupt stream
state. The daemon slows its reads instead and lets TCP apply the backpressure.

## Remote input

Everything a peer sends is untrusted: device names, filenames, MIME types, previews, chat
text, message identifiers, and mDNS records. The daemon strips control characters,
truncates to documented limits, and reduces filenames as above, before any of it reaches a
frontend.

That is sanitisation for the daemon's own safety, not for a frontend's rendering. A client
is still responsible for escaping according to its medium and for not letting a device name
impersonate interface text.

## The control endpoint

On unix the socket is created with mode `0600`. On Windows it is a named pipe with default
permissions.

Anyone who can open it holds the daemon's full authority: accept transfers, read the
clipboard, unpair devices, send files from the daemon's filesystem. There is no
authentication beyond filesystem permissions, and none is planned, because the daemon runs
as the user it acts for.

A stale socket left by a crashed daemon is removed at startup, but only after confirming
nothing is listening on it.

## TLS key logging

Setting `keylog` writes session keys to a path for Wireshark. It is off unless explicitly
configured, warns on startup when on, and writes only where it was told to. It exists so
that developers can inspect encrypted traffic without turning encryption off (§4.2), which
is the better of the two ways to end up reading your own frames.

## Not in scope

- **Internet exposure.** No NAT traversal, no relay, no rendezvous. §1 is explicit, and
  the resource bounds assume an adversary who has to be on your LAN.
- **Traffic analysis.** Frame sizes and timing reveal object sizes and activity. There is
  no padding.
- **Metadata confidentiality against a paired peer.** A paired peer learns device name,
  addresses, and the size and name of everything offered to it.
- **Denial of service by a paired peer.** Rate limits bound the damage; they do not prevent
  a paired device from being a nuisance. `unpair` is the answer.
- **Compromise of the local machine.** The private key is protected by file permissions.
  Anything running as the user can read it, and can also open the control socket.
- **Forward secrecy of stored objects.** Received files are ordinary files.
