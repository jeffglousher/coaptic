use super::{
    CompletedCacheMatch, DATAGRAM_CAPACITY, MessageFailure, Operation, OptionRole, PollError,
    Retry, Wire, has_unsupported_critical, request, request_metadata, request_peer_error,
    same_mid_namespace, send,
};
use crate::message::{Message as CoapMessage, ParsedMessage, Type, decode};
use crate::provisioning::{ConnectionId, Error, Message, PeerError, PeerErrorCode};
use crate::storage::{DatagramIo, Endpoint};

// The one buffer changes from the outgoing request to its terminal response.
// After that transition retransmission is impossible; only an exact CON
// response duplicate can queue another ACK or Reset.
pub(super) struct ErrorRequest {
    operation: Operation,
    wire: Wire,
    retry: Retry,
    expires: u64,
    settled: bool,
    response: bool,
    pending_ack: bool,
    reset: bool,
}

impl ErrorRequest {
    pub(super) fn new(
        operation: Operation,
        peer_id: ConnectionId,
        failure: &MessageFailure,
        initial: u64,
        expires: u64,
    ) -> Result<Self, Error> {
        let message = Message::from_slice(failure.payload())?;
        Ok(Self {
            operation,
            wire: request(operation, peer_id.lakers().as_cbor(), &message)?,
            retry: Retry::new(initial),
            expires,
            settled: false,
            response: false,
            pending_ack: false,
            reset: false,
        })
    }

    const fn unconfirmed(&self, now: u64) -> bool {
        !self.settled
            && now < self.expires
            && match self.retry.wait_until {
                Some(until) => now < until,
                None => true,
            }
    }

    pub(super) const fn in_flight(&self, now: u64) -> bool {
        now < self.expires && (self.pending_ack || self.unconfirmed(now))
    }

    pub(super) fn pending(&self) -> bool {
        self.pending_ack || (!self.settled && self.retry.first_sent.is_none())
    }

    pub(super) fn next_deadline(&self, now: u64) -> u64 {
        if self.pending_ack {
            now
        } else if self.unconfirmed(now) {
            self.retry.deadline(now).min(self.expires)
        } else {
            self.expires
        }
    }

    pub(super) fn classify(
        &self,
        bytes: &[u8],
        parsed: ParsedMessage<'_>,
        now: u64,
    ) -> CompletedCacheMatch {
        if now >= self.expires || bytes.len() > DATAGRAM_CAPACITY || self.retry.first_sent.is_none()
        {
            return CompletedCacheMatch::NoMatch;
        }
        if self.response {
            let saved = decode(self.wire.as_slice()).expect("saved error-request response");
            return if saved.message_id() == parsed.message_id()
                && same_mid_namespace(self.wire.bytes[0], bytes[0])
            {
                if self.wire.as_slice() == bytes {
                    CompletedCacheMatch::ExactDuplicate
                } else {
                    CompletedCacheMatch::MidCollision
                }
            } else {
                CompletedCacheMatch::NoMatch
            };
        }
        let matches = (parsed.is_empty_ack_or_rst() && parsed.message_id() == self.operation.mid)
            || (parsed.code().is_response()
                && parsed.token() == self.operation.token
                && matches!(
                    parsed.ty(),
                    Type::Acknowledgement | Type::Confirmable | Type::NonConfirmable
                )
                && (parsed.ty() != Type::Acknowledgement
                    || parsed.message_id() == self.operation.mid));
        if matches {
            CompletedCacheMatch::ExactDuplicate
        } else {
            CompletedCacheMatch::NoMatch
        }
    }

    pub(super) fn ingest(&mut self, bytes: &[u8], parsed: ParsedMessage<'_>, now: u64) -> bool {
        if self.classify(bytes, parsed, now) != CompletedCacheMatch::ExactDuplicate {
            return false;
        }
        if self.response {
            self.pending_ack |= parsed.ty() == Type::Confirmable;
            return true;
        }
        self.settled = true;
        // An empty ACK stops retries while leaving room for a separate response.
        // A Reset or a complete response is terminal; no EDHOC error is parsed
        // or generated in response to an error request.
        if !parsed.is_empty_ack() {
            self.wire = Wire::copy(bytes).expect("bounded matching response");
            self.response = true;
            self.pending_ack = parsed.ty() == Type::Confirmable;
            self.reset = self.pending_ack && has_unsupported_critical(parsed, OptionRole::Response);
        }
        true
    }

    pub(super) fn flush<T: DatagramIo>(
        &mut self,
        io: &mut T,
        endpoint: Endpoint,
        now: u64,
    ) -> Result<bool, PollError<T::Error>> {
        if now >= self.expires {
            return Ok(false);
        }
        if self.pending_ack {
            let mid = decode(self.wire.as_slice())
                .expect("saved response")
                .message_id();
            let mut bytes = [0; 4];
            let message = if self.reset {
                CoapMessage::empty_rst(mid)
            } else {
                CoapMessage::empty_ack(mid)
            };
            let len = message.encode(&mut bytes).expect("empty response control");
            send(io, endpoint, &bytes[..len])?;
            self.pending_ack = false;
            return Ok(true);
        }
        if self.unconfirmed(now) && self.retry.due(now) {
            if self
                .retry
                .first_sent
                .is_some_and(|first| now > first.saturating_add(self.retry.initial * 15))
            {
                self.retry.next_retry = None;
                return Ok(false);
            }
            let mut next = self.retry;
            if next.sent(now).is_err() {
                self.settled = true;
                return Err(PollError::Clock);
            }
            send(io, endpoint, self.wire.as_slice())?;
            self.retry = next;
            return Ok(true);
        }
        Ok(false)
    }
}

// A late error needs one retained request buffer. Its payload-free response is
// reconstructed from the saved MID/token; the earlier completion replies stay
// in their own slots with their original deadlines.
pub(super) struct ErrorReceipt {
    request: Wire,
    expires: u64,
    pending: bool,
    reported: bool,
}

impl ErrorReceipt {
    pub(super) fn matches_new(bytes: &[u8], local_id: ConnectionId) -> bool {
        if bytes.len() > DATAGRAM_CAPACITY {
            return false;
        }
        let Ok(parsed) = decode(bytes) else {
            return false;
        };
        if !request_metadata(parsed) {
            return false;
        }
        let Ok((id, prefix)) = ConnectionId::decode_prefix(parsed.payload()) else {
            return false;
        };
        id == local_id
            && PeerError::parse(&parsed.payload()[prefix..]).is_some_and(|error| {
                matches!(
                    error.code(),
                    PeerErrorCode::ReservedSuccess | PeerErrorCode::Unspecified
                )
            })
    }

    pub(super) fn new(bytes: &[u8], expires: u64) -> Self {
        Self {
            request: Wire::copy(bytes).expect("bounded admitted error request"),
            expires,
            pending: true,
            reported: false,
        }
    }

    pub(super) fn peer_error(&self) -> PeerError<'_> {
        request_peer_error(self.request.as_slice()).expect("admitted final error")
    }

    pub(super) fn pending(&self) -> bool {
        self.pending
    }

    pub(super) fn next_deadline(&self, now: u64) -> u64 {
        if self.pending || !self.reported {
            now
        } else {
            self.expires
        }
    }

    pub(super) fn classify(&self, bytes: &[u8], now: u64) -> CompletedCacheMatch {
        if now >= self.expires || bytes.len() < 4 {
            return CompletedCacheMatch::NoMatch;
        }
        let saved = self.request.as_slice();
        if saved[2..4] != bytes[2..4] || !same_mid_namespace(saved[0], bytes[0]) {
            return CompletedCacheMatch::NoMatch;
        }
        if saved == bytes {
            CompletedCacheMatch::ExactDuplicate
        } else {
            CompletedCacheMatch::MidCollision
        }
    }

    pub(super) fn queue(&mut self, bytes: &[u8], now: u64) -> bool {
        if self.classify(bytes, now) != CompletedCacheMatch::ExactDuplicate {
            return false;
        }
        self.pending = true;
        true
    }

    pub(super) fn flush<T: DatagramIo>(
        &mut self,
        io: &mut T,
        endpoint: Endpoint,
        now: u64,
    ) -> Result<bool, PollError<T::Error>> {
        if now >= self.expires {
            return Ok(false);
        }
        let progress = self.pending;
        if self.pending {
            let parsed = decode(self.request.as_slice()).expect("saved final error request");
            let mut response = [0; 12];
            let len = CoapMessage::new(
                Type::Acknowledgement,
                crate::message::Code::CHANGED,
                parsed.message_id(),
            )
            .with_token(parsed.token())
            .encode(&mut response)
            .expect("payload-free error receipt");
            send(io, endpoint, &response[..len])?;
            self.pending = false;
        }
        if !self.reported {
            self.reported = true;
            return Err(PollError::PeerAbort(self.peer_error().code()));
        }
        Ok(progress)
    }
}
