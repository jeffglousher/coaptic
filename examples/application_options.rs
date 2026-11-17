//! Ordinary sync/async client calls against an in-memory CoAP peer.
//!
//! Run: `cargo run --example application_options`.
//! This fixture is explicitly unprotected; it is not a network/security example.
//! App currently emits typed standard options. Custom elective response options
//! can be consumed through `received_options()` with `full_responses()` enabled.
//! The wire-level peer below emits that option; App has no custom-option setter.

use coaptic::app::{AsyncCallError, Error};
use coaptic::message::{ContentFormat, Message, Opt, OptionNumber, Type, decode, encode};
use coaptic::platform::AsyncTimer;
use coaptic::storage::{AsyncDatagramIo, DatagramIo, DatagramTransport};
use coaptic::{App, Code, Endpoint, Response};
use core::convert::Infallible;
use core::future::{Future, pending};
use core::task::{Context, Poll, Waker};

const PEER: Endpoint = Endpoint::v4([192, 0, 2, 1], 5683);
// A private elective option for this example, not an IANA assignment.
const RECEIPT: OptionNumber = OptionNumber::new(65000);
const PAYLOAD: &[u8] = b"target=21.5";

struct Peer {
    reply: [u8; 256],
    reply_len: usize,
}

impl DatagramTransport for Peer {
    type Error = &'static str;
}

impl DatagramIo for Peer {
    fn recv(&mut self, output: &mut [u8]) -> Result<Option<(usize, Endpoint)>, Self::Error> {
        if self.reply_len == 0 {
            return Ok(None);
        }
        let n = self.reply_len;
        let target = output.get_mut(..n).ok_or("receive buffer too small")?;
        target.copy_from_slice(&self.reply[..n]);
        self.reply_len = 0;
        Ok(Some((n, PEER)))
    }

    fn send(&mut self, peer: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error> {
        if peer != PEER || self.reply_len != 0 {
            return Err("unexpected peer or unread response");
        }
        let request = decode(bytes).map_err(|_| "malformed request")?;
        if request.code() != Code::PUT
            || request.payload() != PAYLOAD
            || request.content_format() != Some(Ok(ContentFormat::TEXT_PLAIN))
            || request.accept() != Some(Ok(ContentFormat::TEXT_PLAIN))
        {
            return Err("expected PUT with text/plain payload and Accept");
        }
        // Options are in numeric order. Text/plain's integer value is zero,
        // whose canonical CoAP uint encoding is the empty slice.
        let format = ContentFormat::TEXT_PLAIN.encode();
        let options = [
            Opt::content_format(&format),
            Opt::new(RECEIPT, b"applied-1"),
        ];
        let reply = Message::new(Type::Acknowledgement, Code::CHANGED, request.message_id())
            .with_token(request.token())
            .with_options(&options)
            .with_payload(b"target accepted");
        self.reply_len = encode(&reply, &mut self.reply).map_err(|_| "response exceeds buffer")?;
        Ok(bytes.len())
    }
}

impl AsyncDatagramIo for Peer {
    async fn recv(&mut self, output: &mut [u8]) -> Result<(usize, Endpoint), Self::Error> {
        match DatagramIo::recv(self, output)? {
            Some(packet) => Ok(packet),
            None => pending().await,
        }
    }

    async fn send(&mut self, peer: Endpoint, bytes: &[u8]) -> Result<usize, Self::Error> {
        DatagramIo::send(self, peer, bytes)
    }
}

// This fixture completes every operation immediately, with its manual clock
// fixed at zero. Production integrations supply a real timer and executor.
struct FixtureTimer;
impl AsyncTimer for FixtureTimer {
    async fn wait_until(&mut self, deadline_ms: u64) {
        if deadline_ms != 0 {
            pending::<()>().await;
        }
    }
}

fn ready<T>(future: impl Future<Output = Result<T, String>>) -> Result<T, String> {
    let mut future = core::pin::pin!(future);
    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(result) => result,
        Poll::Pending => Err(
            "in-memory fixture unexpectedly waited; use a platform executor for real I/O".into(),
        ),
    }
}

fn explain(error: Error<&'static str>) -> String {
    match error {
        Error::Saturated => "local capacity exhausted: collect completed Calls or cancel abandoned ones before admitting work".into(),
        Error::DeadlineElapsed => "request deadline elapsed: inspect current resource state before retrying a write with an uncertain outcome".into(),
        other => format!("request failed ({other:?}); inspect transport/state before retrying the write"),
    }
}

fn inspect(reply: Response<'_>) -> Result<(), String> {
    if !reply.code().is_success() {
        return Err(format!(
            "peer returned {:?}: inspect payload before retrying",
            reply.code()
        ));
    }
    if reply.format() != Some(ContentFormat::TEXT_PLAIN) {
        return Err("peer returned an unexpected Content-Format; do not decode as text".into());
    }
    let receipt = reply
        .received_options()
        .find(|option| option.number() == RECEIPT)
        .ok_or("missing application receipt: reconcile resource state before retrying")?;
    if receipt.value() != b"applied-1" || reply.payload() != b"target accepted" {
        return Err("unexpected receipt or body: reconcile resource state before retrying".into());
    }
    println!(
        "{} (receipt {})",
        String::from_utf8_lossy(reply.payload()),
        String::from_utf8_lossy(receipt.value())
    );
    Ok(())
}

fn run_sync() -> Result<(), String> {
    let mut app = App::builder()
        .randomness(|bytes| getrandom::fill(bytes).is_ok())
        .full_responses()
        .allow_unprotected()
        .bind(Peer {
            reply: [0; 256],
            reply_len: 0,
        })
        .map_err(|error| format!("cannot build App: {error:?}"))?;
    let call = app
        .put("thermostat/target")
        .to(PEER)
        .payload(PAYLOAD)
        .content_format(ContentFormat::TEXT_PLAIN)
        .accept(ContentFormat::TEXT_PLAIN)
        .deadline(5000)
        .send(0)
        .map_err(explain)?;
    app.poll(0).map_err(explain)?;
    let mut output = [0; 256];
    let reply = app
        .take_response_into(call, &mut output)
        .map_err(|error| {
            format!("response buffer: {error:?}; retry collection with a larger buffer")
        })?
        .ok_or("no response yet: drive App until completion or the request deadline")?
        .map_err(|error| format!("Call failed: {error:?}; reconcile state before retrying"))?;
    inspect(reply)?;
    let refused = app.put("thermostat/target").to(PEER).deadline(0).send(0);
    match refused {
        Err(Error::DeadlineElapsed) => println!("sync: {}", explain(Error::DeadlineElapsed)),
        _ => return Err("expired sync request was not refused".into()),
    }
    Ok(())
}

async fn run_async() -> Result<(), String> {
    let mut app = App::builder()
        .randomness(|bytes| getrandom::fill(bytes).is_ok())
        .full_responses()
        .allow_unprotected()
        .bind(Peer {
            reply: [0; 256],
            reply_len: 0,
        })
        .map_err(|error| format!("cannot build App: {error:?}"))?;
    let mut timer = FixtureTimer;
    let mut clock = || Ok::<_, Infallible>(0);
    let call = app
        .put("thermostat/target")
        .to(PEER)
        .payload(PAYLOAD)
        .content_format(ContentFormat::TEXT_PLAIN)
        .accept(ContentFormat::TEXT_PLAIN)
        .deadline(5000)
        .send_async(0, &mut clock, &mut timer)
        .await
        .map_err(|error| format!("async send: {error:?}; reconcile state before retrying"))?;
    app.wait_async(0, &mut timer, pending())
        .await
        .map_err(explain)?;
    app.progress_async(0, &mut clock, &mut timer, 5000)
        .await
        .map_err(|error| format!("async protocol progress: {error:?}"))?;
    let mut output = [0; 256];
    let reply = app
        .take_response_into(call, &mut output)
        .map_err(|error| {
            format!("response buffer: {error:?}; retry collection with a larger buffer")
        })?
        .ok_or("no response yet: await ingress and drive App until the request deadline")?
        .map_err(|error| format!("Call failed: {error:?}; reconcile state before retrying"))?;
    inspect(reply)?;
    let refused = app
        .put("thermostat/target")
        .to(PEER)
        .deadline(0)
        .send_async(0, &mut clock, &mut timer)
        .await;
    match refused {
        Err(AsyncCallError::Request(Error::DeadlineElapsed)) => {
            println!("async: {}", explain(Error::DeadlineElapsed))
        }
        _ => return Err("expired async request was not refused".into()),
    }
    Ok(())
}

fn main() -> Result<(), String> {
    run_sync()?;
    ready(run_async())
}
