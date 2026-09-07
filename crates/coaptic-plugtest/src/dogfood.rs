//! Timed coaptic ↔ coap-rs dogfood (real UDP, both directions).
//!
//! ```text
//! cargo run -p coaptic-plugtest --bin dogfood
//! cargo run -p coaptic-plugtest --bin dogfood -- --iterations 2
//! ```
//!
//! Jeff’s bar is back-and-forth with an independent stack, plus wall-clock
//! (and Engine `now_ms` deltas on the coaptic client). This is not a
//! plugtest TD grader. Engine metric *counters* print when
//! [`coaptic::storage::Engine`] grows a snapshot API (#125 sibling); this
//! module does not invent a second counter block.

use std::io::{self, Write};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use coaptic::app::DEFAULT_ROUTES;
use coaptic::message::{Code, ContentFormat};
use coaptic::storage::{DatagramIo, Engine, Storage};
use coaptic::{App, Call, Endpoint, profiles};

use crate::coap_rs::CoapRsPeer;
use crate::coaptic::bind_site;
use crate::pcap::bind_loopback;
use crate::peer::{ClientRequest, ClientResponse, Peer, PeerError};
use crate::runner::harness_lock;
use crate::site;

/// How long and how many times the mixed-stack loops run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Config {
    /// Full GET/PUT/POST + Observe-register + block-wise loops per direction.
    pub iterations: usize,
    /// Deadline for a small CON exchange.
    pub timeout: Duration,
    /// Deadline for Block1 / Block2 (large body).
    pub block_timeout: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            iterations: 20,
            timeout: Duration::from_millis(1500),
            block_timeout: Duration::from_millis(4000),
        }
    }
}

impl Config {
    /// Help text for the `dogfood` bin (the one command).
    pub const USAGE: &'static str = "\
cargo run -p coaptic-plugtest --bin dogfood -- [OPTIONS]

Timed coaptic ↔ coap-rs dogfood over loopback UDP.
Both directions: coap-rs client → coaptic server, and coaptic client → coap-rs
server. Each iteration is GET/PUT/POST /test, Observe GET /obs, Block2 GET
/large, Block1 PUT /large-update. Prints wall min/mean/p50/p99/max (and Engine
clock deltas on the coaptic client).

Options:
  --iterations N          loops per direction (default 20)
  --timeout-ms N          small-exchange deadline (default 1500)
  --block-timeout-ms N    Block1/Block2 deadline (default 4000)
  -h, --help              print this message
";

    /// Parse `dogfood` CLI args (`--iterations`, timeouts). Not `--help`.
    pub fn from_args<I, S>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut cfg = Self::default();
        let mut it = args.into_iter();
        while let Some(raw) = it.next() {
            let a = raw.as_ref();
            let (flag, inline) = split_flag(a);
            match flag {
                "--iterations" => {
                    cfg.iterations = parse_usize(flag, inline, &mut it)?;
                }
                "--timeout-ms" => {
                    cfg.timeout = Duration::from_millis(parse_u64(flag, inline, &mut it)?);
                }
                "--block-timeout-ms" => {
                    cfg.block_timeout = Duration::from_millis(parse_u64(flag, inline, &mut it)?);
                }
                other => {
                    return Err(format!(
                        "unknown argument {other:?}\n{}",
                        Self::USAGE.trim_end()
                    ));
                }
            }
        }
        if cfg.iterations == 0 {
            return Err("--iterations must be >= 1".into());
        }
        Ok(cfg)
    }
}

fn split_flag(a: &str) -> (&str, Option<&str>) {
    match a.split_once('=') {
        Some((flag, val)) => (flag, Some(val)),
        None => (a, None),
    }
}

fn parse_usize(
    flag: &str,
    inline: Option<&str>,
    it: &mut impl Iterator<Item = impl AsRef<str>>,
) -> Result<usize, String> {
    parse_u64(flag, inline, it)?
        .try_into()
        .map_err(|_| format!("{flag} is too large"))
}

fn parse_u64(
    flag: &str,
    inline: Option<&str>,
    it: &mut impl Iterator<Item = impl AsRef<str>>,
) -> Result<u64, String> {
    let raw = match inline {
        Some(v) if !v.is_empty() => v.to_owned(),
        Some(_) => return Err(format!("{flag} needs a value")),
        None => it
            .next()
            .map(|s| s.as_ref().to_owned())
            .ok_or_else(|| format!("{flag} needs a value"))?,
    };
    raw.parse::<u64>()
        .map_err(|_| format!("{flag}: expected a number, got {raw:?}"))
}

/// Run both mixed-stack directions and write a timing + Engine snapshot.
pub fn run(cfg: Config, mut out: impl Write) -> Result<(), PeerError> {
    let _guard = harness_lock();
    let t0 = Instant::now();
    writeln!(
        out,
        "dogfood  coaptic ↔ coap-rs  iterations={}  timeout={}ms  block={}ms",
        cfg.iterations,
        cfg.timeout.as_millis(),
        cfg.block_timeout.as_millis()
    )
    .map_err(io_err)?;

    site::reset();
    let server = spawn_coaptic_server()?;
    let dest = server.addr;
    writeln!(out, "\n== coap-rs → coaptic  server={dest}").map_err(io_err)?;
    let a = run_coap_rs_client(&cfg, dest)?;
    a.write("  ", &mut out).map_err(io_err)?;
    let server_line = server.snapshot_line();
    let server_now = server.now_ms();
    drop(server);
    writeln!(
        out,
        "  engine  {server_line}  now_ms={server_now}  (coaptic server)"
    )
    .map_err(io_err)?;

    site::reset();
    let mut rs_server = CoapRsPeer::new();
    let dest = rs_server.start_server()?;
    writeln!(out, "\n== coaptic → coap-rs  server={dest}").map_err(io_err)?;
    let (b, client_line, client_now, caps) = run_coaptic_client(&cfg, dest)?;
    b.write("  ", &mut out).map_err(io_err)?;
    writeln!(
        out,
        "  engine  {client_line}  now_ms={client_now}  (coaptic client)"
    )
    .map_err(io_err)?;
    writeln!(out, "  capacities  {caps}").map_err(io_err)?;
    rs_server.stop_server();

    let wall = t0.elapsed();
    writeln!(out, "\n== integration").map_err(io_err)?;
    writeln!(
        out,
        "  wall     {:.3}s  (bind + {} loops × 2 directions)",
        wall.as_secs_f64(),
        cfg.iterations
    )
    .map_err(io_err)?;
    writeln!(out, "  counters {}", engine_counters_note()).map_err(io_err)?;
    writeln!(out, "dogfood  ok").map_err(io_err)?;
    Ok(())
}

fn io_err(e: io::Error) -> PeerError {
    PeerError(e.to_string())
}

/// Occupancy is always Engine state. Counters come from the #125 metrics PR.
fn engine_counters_note() -> &'static str {
    "pending Engine::metrics snapshot (#125); occupancy above is not a second counter system"
}

/// Format occupancy (and counters when [`Engine`] exposes them).
fn format_engine_snapshot<S: Storage>(engine: &mut Engine<S>) -> String {
    let rx = engine.rx_occupied();
    let tx = engine.tx_occupied();
    let mut line = format!("occupancy rx={rx} tx={tx}");
    if let Some(extra) = engine_metrics_suffix(engine) {
        line.push(' ');
        line.push_str(&extra);
    }
    line
}

/// Hook for the sibling metrics PR. Returns `None` until `Engine::metrics` exists.
fn engine_metrics_suffix<S: Storage>(_engine: &Engine<S>) -> Option<String> {
    None
}

fn format_capacities<S: Storage>(engine: &Engine<S>) -> String {
    let c = engine.capacities();
    format!(
        "rx_dgram={}/{} tx_dgram={}/{} dedup={} observe={} rx_body={:?}x{:?} tx_body={:?}x{:?}",
        c.rx_datagram_slots,
        c.rx_datagram_bytes,
        c.tx_datagram_slots,
        c.tx_datagram_bytes,
        c.dedup_entries,
        c.observe_entries,
        c.rx_body_slots,
        c.rx_body_bytes,
        c.tx_body_slots,
        c.tx_body_bytes
    )
}

type ClientApp<T> = App<profiles::Default, T, DEFAULT_ROUTES, true>;

struct CoapticServer {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    snapshot: Arc<Mutex<ServerSnap>>,
    join: Option<JoinHandle<()>>,
}

struct ServerSnap {
    line: String,
    now_ms: u64,
}

impl CoapticServer {
    fn snapshot_line(&self) -> String {
        self.snapshot.lock().expect("snap").line.clone()
    }

    fn now_ms(&self) -> u64 {
        self.snapshot.lock().expect("snap").now_ms
    }
}

impl Drop for CoapticServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

fn spawn_coaptic_server() -> Result<CoapticServer, PeerError> {
    let (sock, addr) = bind_loopback().map_err(|e| e.to_string())?;
    let stop = Arc::new(AtomicBool::new(false));
    let snapshot = Arc::new(Mutex::new(ServerSnap {
        line: String::from("occupancy rx=? tx=?"),
        now_ms: 0,
    }));
    let stop_t = Arc::clone(&stop);
    let snap_t = Arc::clone(&snapshot);
    let join = thread::Builder::new()
        .name("coaptic-dogfood-server".into())
        .spawn(move || {
            let mut app = bind_site(sock);
            let origin = Instant::now();
            while !stop_t.load(Ordering::SeqCst) {
                let now = elapsed_ms(origin);
                let _ = app.poll(now);
                let line = format_engine_snapshot(app.engine_mut());
                *snap_t.lock().expect("snap") = ServerSnap { line, now_ms: now };
                thread::yield_now();
            }
            let now = elapsed_ms(origin);
            let line = format_engine_snapshot(app.engine_mut());
            *snap_t.lock().expect("snap") = ServerSnap { line, now_ms: now };
        })
        .map_err(|e| e.to_string())?;
    thread::sleep(Duration::from_millis(5));
    Ok(CoapticServer {
        addr,
        stop,
        snapshot,
        join: Some(join),
    })
}

fn elapsed_ms(origin: Instant) -> u64 {
    u64::try_from(origin.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[derive(Clone, Debug)]
struct Got {
    code: Code,
    payload: Vec<u8>,
    body: Option<Vec<u8>>,
    observe: Option<u32>,
}

impl From<ClientResponse> for Got {
    fn from(r: ClientResponse) -> Self {
        Self {
            code: r.code,
            payload: r.payload,
            body: r.body,
            observe: r.observe,
        }
    }
}

impl Got {
    fn assembled_len(&self) -> usize {
        self.body.as_ref().map_or(self.payload.len(), Vec::len)
    }
}

struct Series {
    label: &'static str,
    wall_us: Vec<u64>,
    engine_ms: Vec<u64>,
}

impl Series {
    fn new(label: &'static str) -> Self {
        Self {
            label,
            wall_us: Vec::new(),
            engine_ms: Vec::new(),
        }
    }

    fn record(&mut self, wall: Duration, engine_ms: Option<u64>) {
        self.wall_us
            .push(u64::try_from(wall.as_micros()).unwrap_or(u64::MAX));
        if let Some(ms) = engine_ms {
            self.engine_ms.push(ms);
        }
    }

    fn write(&self, indent: &str, out: &mut impl Write) -> io::Result<()> {
        let wall = summarize(&self.wall_us);
        write!(
            out,
            "{indent}{:<28} n={:<3}  wall ms  min={:.3} mean={:.3} p50={:.3} p99={:.3} max={:.3}",
            self.label,
            self.wall_us.len(),
            us_ms(wall.min),
            wall.mean / 1000.0,
            us_ms(wall.p50),
            us_ms(wall.p99),
            us_ms(wall.max)
        )?;
        if !self.engine_ms.is_empty() {
            let eng = summarize(&self.engine_ms);
            if eng.max == 0 {
                write!(out, "  engineΔ ms  all <1")?;
            } else {
                write!(
                    out,
                    "  engineΔ ms  min={} mean={:.1} max={}",
                    eng.min, eng.mean, eng.max
                )?;
            }
        }
        writeln!(out)
    }
}

struct PairReport {
    get: Series,
    put: Series,
    post: Series,
    obs: Series,
    block2: Series,
    block1: Series,
    loop_: Series,
}

impl PairReport {
    fn new() -> Self {
        Self {
            get: Series::new("GET /test"),
            put: Series::new("PUT /test"),
            post: Series::new("POST /test"),
            obs: Series::new("OBS GET /obs"),
            block2: Series::new("BLOCK2 GET /large"),
            block1: Series::new("BLOCK1 PUT /large-update"),
            loop_: Series::new("LOOP (all verbs)"),
        }
    }

    fn write(&self, indent: &str, out: &mut impl Write) -> io::Result<()> {
        self.get.write(indent, out)?;
        self.put.write(indent, out)?;
        self.post.write(indent, out)?;
        self.obs.write(indent, out)?;
        self.block2.write(indent, out)?;
        self.block1.write(indent, out)?;
        self.loop_.write(indent, out)
    }
}

struct Summary {
    min: u64,
    mean: f64,
    max: u64,
    p50: u64,
    p99: u64,
}

fn summarize(samples: &[u64]) -> Summary {
    if samples.is_empty() {
        return Summary {
            min: 0,
            mean: 0.0,
            max: 0,
            p50: 0,
            p99: 0,
        };
    }
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let sum: u128 = sorted.iter().copied().map(u128::from).sum();
    let mean = sum as f64 / sorted.len() as f64;
    Summary {
        min: *sorted.first().unwrap_or(&0),
        mean,
        max: *sorted.last().unwrap_or(&0),
        p50: percentile(&sorted, 50),
        p99: percentile(&sorted, 99),
    }
}

/// Nearest-rank: p99 on a short run is the last sample.
fn percentile(sorted: &[u64], p: u8) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let n = sorted.len();
    let rank = (usize::from(p) * n).div_ceil(100).max(1);
    sorted[rank - 1]
}

fn us_ms(us: u64) -> f64 {
    us as f64 / 1000.0
}

fn run_coap_rs_client(cfg: &Config, dest: SocketAddr) -> Result<PairReport, PeerError> {
    let mut client = CoapRsPeer::new();
    let mut report = PairReport::new();
    let large = site::large_body();
    for i in 0..cfg.iterations {
        let loop_t0 = Instant::now();
        one_rs(
            &mut client,
            dest,
            &mut report.get,
            get_test(),
            cfg.timeout,
            &[Code::CONTENT],
            None,
            i,
            "GET /test",
        )?;
        one_rs(
            &mut client,
            dest,
            &mut report.put,
            put_test(),
            cfg.timeout,
            &[Code::CHANGED],
            None,
            i,
            "PUT /test",
        )?;
        one_rs(
            &mut client,
            dest,
            &mut report.post,
            post_test(),
            cfg.timeout,
            &[Code::CREATED, Code::CHANGED],
            None,
            i,
            "POST /test",
        )?;
        one_rs(
            &mut client,
            dest,
            &mut report.obs,
            get_obs(),
            cfg.timeout,
            &[Code::CONTENT],
            Some(true),
            i,
            "OBS GET /obs",
        )?;
        one_rs(
            &mut client,
            dest,
            &mut report.block2,
            get_large(cfg.block_timeout),
            cfg.block_timeout,
            &[Code::CONTENT],
            Some(false),
            i,
            "BLOCK2 GET /large",
        )?;
        let mut put_large = ClientRequest::request(Code::PUT, &["large-update"]);
        put_large.payload = large.clone();
        put_large.content_format = Some(0);
        put_large.timeout = cfg.block_timeout;
        one_rs(
            &mut client,
            dest,
            &mut report.block1,
            put_large,
            cfg.block_timeout,
            &[Code::CHANGED],
            None,
            i,
            "BLOCK1 PUT /large-update",
        )?;
        report.loop_.record(loop_t0.elapsed(), None);
    }
    Ok(report)
}

#[allow(clippy::too_many_arguments)]
fn one_rs(
    client: &mut CoapRsPeer,
    dest: SocketAddr,
    series: &mut Series,
    mut req: ClientRequest,
    timeout: Duration,
    want: &[Code],
    observe_or_block2: Option<bool>,
    iter: usize,
    label: &str,
) -> Result<(), PeerError> {
    req.timeout = timeout;
    let t0 = Instant::now();
    let got = Got::from(client.send_request(dest, &req)?);
    series.record(t0.elapsed(), None);
    expect_codes(label, iter, got.code, want)?;
    match observe_or_block2 {
        Some(true) => {
            if got.observe.is_none() {
                return Err(PeerError(format!(
                    "iteration {iter} {label}: missing Observe on {code}",
                    code = got.code
                )));
            }
        }
        Some(false) => {
            let n = got.assembled_len();
            if n < 64 {
                return Err(PeerError(format!(
                    "iteration {iter} {label}: assembled only {n} bytes"
                )));
            }
        }
        None => {}
    }
    Ok(())
}

fn get_test() -> ClientRequest {
    ClientRequest::get(&["test"])
}

fn put_test() -> ClientRequest {
    let mut r = ClientRequest::request(Code::PUT, &["test"]);
    r.payload = site::TEST_BODY.to_vec();
    r.content_format = Some(0);
    r
}

fn post_test() -> ClientRequest {
    let mut r = ClientRequest::request(Code::POST, &["test"]);
    r.payload = site::TEST_BODY.to_vec();
    r.content_format = Some(0);
    r
}

fn get_obs() -> ClientRequest {
    let mut r = ClientRequest::get(&["obs"]);
    r.observe = Some(0);
    r
}

fn get_large(timeout: Duration) -> ClientRequest {
    let mut r = ClientRequest::get(&["large"]);
    r.timeout = timeout;
    r
}

fn run_coaptic_client(
    cfg: &Config,
    dest: SocketAddr,
) -> Result<(PairReport, String, u64, String), PeerError> {
    let (sock, _) = bind_loopback().map_err(|e| e.to_string())?;
    let mut app = App::profile::<profiles::Default>()
        .block_wise::<true>()
        .bind(sock)
        .map_err(|e| format!("bind: {e}"))?;
    let origin = Instant::now();
    let peer = Endpoint::from(dest);
    let caps = format_capacities(app.engine());
    let mut report = PairReport::new();
    let large = site::large_body();

    for i in 0..cfg.iterations {
        let loop_t0 = Instant::now();
        let loop_e0 = elapsed_ms(origin);

        timed_call(
            &mut app,
            origin,
            cfg.timeout,
            &mut report.get,
            |app, now| {
                app.get("test")
                    .to(peer)
                    .send(now)
                    .map_err(|e| format!("send GET /test: {e}").into())
            },
            |got| {
                expect_codes("GET /test", i, got.code, &[Code::CONTENT])?;
                if got.payload != site::TEST_BODY {
                    return Err(PeerError(format!(
                        "iteration {i} GET /test: payload {} bytes, expected {}",
                        got.payload.len(),
                        site::TEST_BODY.len()
                    )));
                }
                Ok(())
            },
        )?;

        timed_call(
            &mut app,
            origin,
            cfg.timeout,
            &mut report.put,
            |app, now| {
                app.put("test")
                    .payload(site::TEST_BODY)
                    .content_format(ContentFormat::TEXT_PLAIN)
                    .to(peer)
                    .send(now)
                    .map_err(|e| format!("send PUT /test: {e}").into())
            },
            |got| expect_codes("PUT /test", i, got.code, &[Code::CHANGED]),
        )?;

        timed_call(
            &mut app,
            origin,
            cfg.timeout,
            &mut report.post,
            |app, now| {
                app.post("test")
                    .payload(site::TEST_BODY)
                    .content_format(ContentFormat::TEXT_PLAIN)
                    .to(peer)
                    .send(now)
                    .map_err(|e| format!("send POST /test: {e}").into())
            },
            |got| expect_codes("POST /test", i, got.code, &[Code::CREATED, Code::CHANGED]),
        )?;

        timed_call(
            &mut app,
            origin,
            cfg.timeout,
            &mut report.obs,
            |app, now| {
                app.get("obs")
                    .observe()
                    .to(peer)
                    .send(now)
                    .map_err(|e| format!("send OBS GET /obs: {e}").into())
            },
            |got| {
                expect_codes("OBS GET /obs", i, got.code, &[Code::CONTENT])?;
                if got.observe.is_none() {
                    return Err(PeerError(format!(
                        "iteration {i} OBS GET /obs: missing Observe"
                    )));
                }
                Ok(())
            },
        )?;
        // Drop the registration so the next loop does not fill Observe slots.
        let now = elapsed_ms(origin).saturating_add(1);
        let off = app
            .get("obs")
            .deregister()
            .to(peer)
            .send(now)
            .map_err(|e| format!("send OBS deregister: {e}"))?;
        wait_call(&mut app, off, origin, cfg.timeout)?;

        timed_call(
            &mut app,
            origin,
            cfg.block_timeout,
            &mut report.block2,
            |app, now| {
                app.get("large")
                    .to(peer)
                    .send(now)
                    .map_err(|e| format!("send GET /large: {e}").into())
            },
            |got| {
                expect_codes("BLOCK2 GET /large", i, got.code, &[Code::CONTENT])?;
                let n = got.assembled_len();
                if n != site::LARGE_LEN {
                    return Err(PeerError(format!(
                        "iteration {i} BLOCK2 GET /large: assembled {n}, expected {}",
                        site::LARGE_LEN
                    )));
                }
                Ok(())
            },
        )?;

        timed_call(
            &mut app,
            origin,
            cfg.block_timeout,
            &mut report.block1,
            |app, now| {
                app.put("large-update")
                    .payload(&large)
                    .content_format(ContentFormat::TEXT_PLAIN)
                    .to(peer)
                    .send(now)
                    .map_err(|e| format!("send PUT /large-update: {e}").into())
            },
            |got| expect_codes("BLOCK1 PUT /large-update", i, got.code, &[Code::CHANGED]),
        )?;

        report.loop_.record(
            loop_t0.elapsed(),
            Some(elapsed_ms(origin).saturating_sub(loop_e0)),
        );
    }

    let now = elapsed_ms(origin);
    let line = format_engine_snapshot(app.engine_mut());
    Ok((report, line, now, caps))
}

fn timed_call<T, F, C>(
    app: &mut ClientApp<T>,
    origin: Instant,
    timeout: Duration,
    series: &mut Series,
    send: F,
    check: C,
) -> Result<(), PeerError>
where
    T: DatagramIo<Error = std::io::Error>,
    F: FnOnce(&mut ClientApp<T>, u64) -> Result<Call, PeerError>,
    C: FnOnce(&Got) -> Result<(), PeerError>,
{
    let e0 = elapsed_ms(origin);
    let t0 = Instant::now();
    let now = e0.saturating_add(1);
    let call = send(app, now)?;
    let got = wait_call(app, call, origin, timeout)?;
    let e1 = elapsed_ms(origin);
    series.record(t0.elapsed(), Some(e1.saturating_sub(e0)));
    check(&got)
}

fn wait_call<T: DatagramIo<Error = std::io::Error>>(
    app: &mut ClientApp<T>,
    call: Call,
    origin: Instant,
    timeout: Duration,
) -> Result<Got, PeerError> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let now = elapsed_ms(origin).saturating_add(1);
        app.poll(now).map_err(|e| format!("poll: {e}"))?;
        if let Some(resp) = app.take_response(call) {
            return Ok(Got {
                code: resp.code(),
                payload: resp.payload().to_vec(),
                body: resp.body().map(ToOwned::to_owned),
                observe: resp.observe_seq(),
            });
        }
        thread::yield_now();
    }
    Err(PeerError("timeout waiting for App take_response".into()))
}

fn expect_codes(label: &str, iter: usize, got: Code, want: &[Code]) -> Result<(), PeerError> {
    if want.contains(&got) {
        Ok(())
    } else {
        Err(PeerError(format!(
            "iteration {iter} {label}: got {got}, expected one of {want:?}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::{Config, percentile, run};

    #[test]
    fn percentile_ranks() {
        let s = [1u64, 2, 3, 4];
        assert_eq!(percentile(&s, 0), 1);
        assert_eq!(percentile(&s, 50), 2);
        assert_eq!(percentile(&s, 99), 4);
        assert_eq!(percentile(&[], 50), 0);
    }

    #[test]
    fn config_parses_flags() {
        let cfg = Config::from_args(["--iterations", "3", "--timeout-ms=200"]).unwrap();
        assert_eq!(cfg.iterations, 3);
        assert_eq!(cfg.timeout.as_millis(), 200);
    }

    #[test]
    fn timed_dogfood_smoke() {
        let mut buf = Vec::new();
        let cfg = Config {
            iterations: 2,
            ..Config::default()
        };
        run(cfg, &mut buf).expect("dogfood smoke");
        let s = String::from_utf8_lossy(&buf);
        assert!(s.contains("coap-rs → coaptic"), "{s}");
        assert!(s.contains("coaptic → coap-rs"), "{s}");
        assert!(s.contains("LOOP (all verbs)"), "{s}");
        assert!(s.contains("dogfood  ok"), "{s}");
        eprintln!("{s}");
    }
}
