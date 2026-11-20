# Windows lost piggybacked response recovery

All 16 single-loss trials recover and complete a fresh read on the same
client and server processes. Both established OSCORE and unprotected
reads recover through the native client's CON retry. The retry request
and the replayed server reply match the original datagrams byte for byte.

## Matched setup

A bounded IPv4 loopback relay forwards C1 CON reads between the frozen
native client and the frozen Mio or Tokio async server. The 64-byte and
4,096-byte responses use the same fixed Device capacities as the
[two-endpoint peer fixture](WINDOWS-PAIRED-PEERS-20261119.md). The client
uses a nine-second deadline. Each pair of reads uses one server process
and a fresh established OSCORE context in the protected cells.

An eight-cell clean control completes the same two reads without loss.
Two reversed passes each cover all eight runtime, security and body
cells. The relay drops only the first read's final piggybacked response,
after confirming its outer MID and token match the request. It retains
only packet lengths, headers, SHA-256 digests and timings. The native
client verifies the complete response and authentication. The
[numeric receipt](WINDOWS-PROTECTED-RESPONSE-RECOVERY-20261120.json)
binds the sources and all 24 raw trials. All owned child processes stop
after each trial.

## Result

All 48 reads complete. Each loss adds one exact request retry. The server
answers with the original reply bytes, and the client completes the new
read with the same process and session. The measured relay intervals from
drop to retry range from 2.004 to 2.950 seconds. No operation fails,
expires or is cancelled in these windows. This is a one-loss read test.
The preceding assurance for effects, exhaustion, cancellation and
expiry remains independently scoped.

The pinned libcoap implementation sends an empty ACK and retransmittable
separate CON for a successful OSCORE response, explicitly to recover a
lost response. This Coaptic test exercises the client-driven retry and
server replay path for piggybacking. It supplies recovery evidence for
the wire behavior seen in the four-endpoint comparison. The timing is
relay-observed recovery latency. The original CPU targets and physical
qualification remain open.
[Pinned libcoap implementation](https://github.com/obgm/libcoap/blob/851533c3cf63d16984d370ce39d586ecb3694971/src/coap_oscore.c#L667-L689)
