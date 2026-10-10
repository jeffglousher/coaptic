//! The same fixed-storage protected service for a device or host transport.
//!
//! This is platform-independent application code, not ESP32 firmware. The caller
//! supplies bounded datagram I/O, secure randomness and a fresh or safely restored
//! OSCORE context, then drives `poll` with a monotonic millisecond clock. Retained
//! credentials need durable sender reservations and replay checkpoints; see
//! `SECURITY.md`. The host demonstration uses fresh credentials for each run.

use coaptic::{
    App, BuildError, Code, Request, Response, app::RandomSource, get, oscore::SecurityContext,
    post, profiles,
};

pub const TELEMETRY: [u8; 200] = [b'x'; 200];

pub fn telemetry(_: Request<'_>) -> Response<'static> {
    Response::content(&TELEMETRY)
}

pub fn echo(request: Request<'_>) -> Response<'static> {
    Response::try_content_copy(request.payload())
        .unwrap_or_else(|_| Response::new(Code::REQUEST_ENTITY_TOO_LARGE))
}

/// Bind fixed default-profile storage without enabling body pools or allocation.
/// No plaintext opt-out is present: the supplied context protects both routes.
pub fn server<T>(
    io: T,
    context: SecurityContext,
    secure_random: RandomSource,
) -> Result<App<profiles::Default, T>, BuildError> {
    App::builder()
        .randomness(secure_random)
        .oscore(context)
        .route("telemetry", get(telemetry))
        .route("echo", post(echo))
        .bind(io)
}

#[cfg(test)]
mod tests {
    use super::*;
    use coaptic::{Endpoint, oscore::DeriveParams, storage::DatagramIo};

    struct Wire {
        incoming: [u8; 1472],
        received: usize,
        outgoing: [u8; 1472],
        sent: usize,
        peer: Endpoint,
    }

    impl Wire {
        fn new(peer: Endpoint) -> Self {
            Self {
                incoming: [0; 1472],
                received: 0,
                outgoing: [0; 1472],
                sent: 0,
                peer,
            }
        }

        fn transfer_to(&mut self, other: &mut Self) {
            assert!(self.sent > 0);
            other.incoming[..self.sent].copy_from_slice(&self.outgoing[..self.sent]);
            other.received = self.sent;
            self.sent = 0;
        }
    }

    impl DatagramIo for Wire {
        type Error = core::convert::Infallible;

        fn recv(&mut self, buf: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
            if self.received == 0 {
                return Ok(None);
            }
            let n = self.received;
            buf[..n].copy_from_slice(&self.incoming[..n]);
            self.received = 0;
            Ok(Some((n, self.peer)))
        }

        fn send(&mut self, _: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error> {
            assert!(self.sent == 0, "fixture admits one datagram at a time");
            self.outgoing[..bytes.len()].copy_from_slice(bytes);
            self.sent = bytes.len();
            Ok(bytes.len())
        }
    }

    fn context(sender: &[u8], recipient: &[u8]) -> SecurityContext {
        // Public fixture credentials are only used in this in-memory test.
        SecurityContext::derive(DeriveParams {
            master_secret: &[0x42; 32],
            master_salt: &[],
            sender_id: sender,
            recipient_id: recipient,
            id_context: &[],
        })
        .unwrap()
    }

    fn random(bytes: &mut [u8]) -> bool {
        bytes.fill(0x35);
        true
    }

    #[test]
    fn shared_service_refuses_unavailable_entropy() {
        assert!(matches!(
            server((), context(&[2], &[1]), |_| false),
            Err(BuildError::RandomnessUnavailable)
        ));
    }

    #[test]
    fn protected_echo_accepts_exact_capacity_and_refuses_one_extra_byte() {
        for len in [128, 129] {
            let server_peer = Endpoint::v4([192, 0, 2, 1], 5683);
            let client_peer = Endpoint::v4([192, 0, 2, 2], 5683);
            let mut service = server(Wire::new(client_peer), context(&[2], &[1]), random).unwrap();
            let mut client = App::builder()
                .randomness(random)
                .oscore(context(&[1], &[2]))
                .full_responses()
                .bind(Wire::new(server_peer))
                .unwrap();
            let payload = [b'a'; 129];
            let call = client
                .post("echo")
                .payload(&payload[..len])
                .to(server_peer)
                .send(0)
                .unwrap();
            client.transport_mut().transfer_to(service.transport_mut());
            service.poll(0).unwrap();
            service.transport_mut().transfer_to(client.transport_mut());
            client.poll(0).unwrap();
            let mut body = [0; 129];
            let reply = client
                .take_response_into(call, &mut body)
                .unwrap()
                .unwrap()
                .unwrap();
            if len == 128 {
                assert_eq!(reply.code(), Code::CONTENT);
                assert_eq!(reply.payload(), &payload[..128]);
            } else {
                assert_eq!(reply.code(), Code::REQUEST_ENTITY_TOO_LARGE);
                assert!(reply.payload().is_empty());
            }
        }
    }
}
