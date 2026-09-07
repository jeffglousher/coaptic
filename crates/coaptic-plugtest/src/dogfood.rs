//! Timed coaptic ↔ coap-rs dogfood, plus a coaptic↔coaptic Observe notify collect.
//!
//! ```text
//! cargo run -p coaptic-plugtest --bin dogfood
//! cargo run -p coaptic-plugtest --bin dogfood -- --iterations 2
//! cargo run -p coaptic-plugtest --bin dogfood -- --json dogfood.json
//! ```
//!
//! Mixed-stack loops stay GET/PUT/POST, Observe **register**, and Block1/Block2
//! against coap-rs (that peer is a register/deregister stub). The notify
//! leg is coaptic-server [`App::notify`] collected by a coaptic-client
//! [`App::take_response`] so [`Metrics::observe_notify`] is not left cold.
//!
//! [`App::reset_metrics`] / [`Engine::reset_metrics`] run at the start of
//! each timed window (`progress` counts idle poll ticks). After each
//! window it prints occupancy and [`coaptic::Metrics`] from
//! [`App::metrics`] / [`Engine::metrics`]. Optional `--json PATH` writes
//! the same numbers (host/load specific; not a CI golden).

use std::io::{self, Write};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use coaptic::app::DEFAULT_ROUTES;
use coaptic::message::{Code, ContentFormat};
use coaptic::storage::{DatagramIo, Engine, Storage};
use coaptic::{App, Call, Endpoint, Metrics, Response, profiles};
use serde::Serialize;

use crate::coap_rs::CoapRsPeer;
use crate::coaptic::bind_site;
use crate::pcap::bind_loopback;
use crate::peer::{ClientRequest, ClientResponse, NotifyMailbox, Peer, PeerError};
use crate::runner::harness_lock;
use crate::site;

/// How long and how many times the mixed-stack and notify-collect loops run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Config {
    /// Full GET/PUT/POST + Observe-register + block-wise loops per direction,
    /// and Observe notify collects on the coaptic↔coaptic leg.
    pub iterations: usize,
    /// Deadline for a small CON exchange.
    pub timeout: Duration,
    /// Deadline for Block1 / Block2 (large body).
    pub block_timeout: Duration,
    /// Write a JSON report here after a successful run (`None` = stdout only).
    pub json_path: Option<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            iterations: 50,
            timeout: Duration::from_millis(1500),
            block_timeout: Duration::from_millis(4000),
            json_path: None,
        }
    }
}

impl Config {
    /// Help text for the `dogfood` bin (the one command).
    pub const USAGE: &'static str = "\
cargo run -p coaptic-plugtest --bin dogfood -- [OPTIONS]

Timed dogfood over loopback UDP.
Mixed stack (both directions): coap-rs client → coaptic server, and coaptic
client → coap-rs server. Each iteration is GET/PUT/POST /test, Observe register
GET /obs, Block2 GET /large, Block1 PUT /large-update.
Notify collect (coaptic ↔ coaptic): register /obs, App::notify, collect the
notification, deregister — so observe_notify is not left cold. coap-rs stays
on the other verbs; it does not collect notifies.

Prints wall min/mean/p50/p99/max (and Engine clock deltas on the coaptic
client). Resets `app.metrics()` around each timed window, then prints the
snapshot. Timings are host/load specific — not a CI golden.

Options:
  --iterations N          loops per direction + notify collects (default 50)
  --timeout-ms N          small-exchange deadline (default 1500)
  --block-timeout-ms N    Block1/Block2 deadline (default 4000)
  --json PATH             write the same numbers as JSON
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
                "--json" => {
                    cfg.json_path = Some(PathBuf::from(parse_string(flag, inline, &mut it)?));
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
    let raw = parse_string(flag, inline, it)?;
    raw.parse::<u64>()
        .map_err(|_| format!("{flag}: expected a number, got {raw:?}"))
}

fn parse_string(
    flag: &str,
    inline: Option<&str>,
    it: &mut impl Iterator<Item = impl AsRef<str>>,
) -> Result<String, String> {
    match inline {
        Some(v) if !v.is_empty() => Ok(v.to_owned()),
        Some(_) => Err(format!("{flag} needs a value")),
        None => it
            .next()
            .map(|s| s.as_ref().to_owned())
            .ok_or_else(|| format!("{flag} needs a value")),
    }
}

/// Run mixed-stack directions, the notify collect, and write timing + metrics.
pub fn run(cfg: Config, mut out: impl Write) -> Result<(), PeerError> {
    let _guard = harness_lock();
    let t0 = Instant::now();
    writeln!(
        out,
        "dogfood  coaptic ↔ coap-rs + notify collect  iterations={}  timeout={}ms  block={}ms",
        cfg.iterations,
        cfg.timeout.as_millis(),
        cfg.block_timeout.as_millis()
    )
    .map_err(io_err)?;

    site::reset();
    let server = spawn_coaptic_server()?;
    let dest = server.addr;
    writeln!(out, "\n== coap-rs → coaptic  server={dest}").map_err(io_err)?;
    server.reset_metrics()?;
    let a = run_coap_rs_client(&cfg, dest)?;
    a.write("  ", &mut out).map_err(io_err)?;
    let rs_to_coaptic = server.snapshot();
    drop(server);
    writeln!(
        out,
        "  engine    {}  now_ms={}  (coaptic server)",
        rs_to_coaptic.occupancy, rs_to_coaptic.now_ms
    )
    .map_err(io_err)?;
    writeln!(out, "  app.metrics()  {}", rs_to_coaptic.metrics).map_err(io_err)?;

    site::reset();
    let mut rs_server = CoapRsPeer::new();
    let dest = rs_server.start_server()?;
    writeln!(out, "\n== coaptic → coap-rs  server={dest}").map_err(io_err)?;
    let (b, client_occ, client_metrics, client_now, caps) = run_coaptic_client(&cfg, dest)?;
    b.write("  ", &mut out).map_err(io_err)?;
    writeln!(
        out,
        "  engine    {client_occ}  now_ms={client_now}  (coaptic client)"
    )
    .map_err(io_err)?;
    writeln!(out, "  app.metrics()  {client_metrics}").map_err(io_err)?;
    writeln!(out, "  capacities  {caps}").map_err(io_err)?;
    rs_server.stop_server();

    site::reset();
    let server = spawn_coaptic_server()?;
    let dest = server.addr;
    writeln!(out, "\n== coaptic ↔ coaptic  observe notify  server={dest}").map_err(io_err)?;
    server.reset_metrics()?;
    let notify = run_coaptic_observe_notify(&cfg, dest, &server)?;
    notify.write("  ", &mut out).map_err(io_err)?;
    let notify_server = server.snapshot();
    drop(server);
    writeln!(
        out,
        "  engine    {}  now_ms={}  (coaptic server)",
        notify_server.occupancy, notify_server.now_ms
    )
    .map_err(io_err)?;
    writeln!(out, "  app.metrics()  {}", notify_server.metrics).map_err(io_err)?;
    writeln!(
        out,
        "  client    {}  now_ms={}  (coaptic client)",
        notify.client_occupancy, notify.client_now_ms
    )
    .map_err(io_err)?;
    writeln!(out, "  client app.metrics()  {}", notify.client_metrics).map_err(io_err)?;
    if notify_server.metrics.observe_notify == 0 || notify.collected == 0 {
        return Err(PeerError(format!(
            "observe_notify stayed cold: collected={} server observe_notify={}",
            notify.collected, notify_server.metrics.observe_notify
        )));
    }
    if notify.collected != cfg.iterations {
        return Err(PeerError(format!(
            "observe notify collected {} of {} iterations",
            notify.collected, cfg.iterations
        )));
    }

    let wall = t0.elapsed();
    writeln!(out, "\n== integration").map_err(io_err)?;
    writeln!(
        out,
        "  wall     {:.3}s  (bind + {} loops × 2 mixed + {} notify collects)",
        wall.as_secs_f64(),
        cfg.iterations,
        cfg.iterations
    )
    .map_err(io_err)?;
    writeln!(
        out,
        "  observe  collected={}  server observe_notify={}",
        notify.collected, notify_server.metrics.observe_notify
    )
    .map_err(io_err)?;
    writeln!(
        out,
        "  metrics  app.metrics() after timed window (reset_metrics around it)"
    )
    .map_err(io_err)?;
    writeln!(
        out,
        "  caveat   timings are host/load specific; not a CI golden"
    )
    .map_err(io_err)?;
    writeln!(out, "dogfood  ok").map_err(io_err)?;

    if let Some(path) = cfg.json_path.as_ref() {
        write_json_report(
            path,
            &cfg,
            wall,
            &a,
            &rs_to_coaptic,
            &b,
            &client_occ,
            &client_metrics,
            client_now,
            &caps,
            &notify,
            &notify_server,
        )?;
        writeln!(out, "dogfood  json  {}", path.display()).map_err(io_err)?;
    }
    Ok(())
}

fn io_err(e: io::Error) -> PeerError {
    PeerError(e.to_string())
}

fn occupancy_line<S: Storage>(engine: &mut Engine<S>) -> String {
    let rx = engine.rx_occupied();
    let tx = engine.tx_occupied();
    format!("occupancy rx={rx} tx={tx}")
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
    reset: Arc<AtomicBool>,
    reset_done: Arc<AtomicBool>,
    notify: NotifyMailbox,
    snapshot: Arc<Mutex<ServerSnap>>,
    join: Option<JoinHandle<()>>,
}

#[derive(Clone)]
struct ServerSnap {
    occupancy: String,
    metrics: Metrics,
    now_ms: u64,
}

impl CoapticServer {
    fn reset_metrics(&self) -> Result<(), PeerError> {
        self.reset_done.store(false, Ordering::SeqCst);
        self.reset.store(true, Ordering::SeqCst);
        let deadline = Instant::now() + Duration::from_millis(200);
        while Instant::now() < deadline {
            if self.reset_done.load(Ordering::SeqCst) {
                return Ok(());
            }
            thread::yield_now();
        }
        Err(PeerError("coaptic server did not ack reset_metrics".into()))
    }

    fn snapshot(&self) -> ServerSnap {
        self.snapshot.lock().expect("snap").clone()
    }

    fn notify(&self, path: &[&str], payload: &[u8]) -> Result<(), PeerError> {
        *self.notify.lock().expect("notify") = Some((
            path.iter().map(|s| (*s).to_owned()).collect(),
            payload.to_vec(),
        ));
        Ok(())
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
    let reset = Arc::new(AtomicBool::new(false));
    let reset_done = Arc::new(AtomicBool::new(false));
    let notify: NotifyMailbox = Arc::new(Mutex::new(None));
    let snapshot = Arc::new(Mutex::new(ServerSnap {
        occupancy: String::from("occupancy rx=? tx=?"),
        metrics: Metrics::ZERO,
        now_ms: 0,
    }));
    let stop_t = Arc::clone(&stop);
    let reset_t = Arc::clone(&reset);
    let reset_done_t = Arc::clone(&reset_done);
    let notify_t = Arc::clone(&notify);
    let snap_t = Arc::clone(&snapshot);
    let join = thread::Builder::new()
        .name("coaptic-dogfood-server".into())
        .spawn(move || {
            let mut app = bind_site(sock);
            let origin = Instant::now();
            let mut pending: Option<(Vec<String>, Vec<u8>)> = None;
            while !stop_t.load(Ordering::SeqCst) {
                if reset_t.swap(false, Ordering::SeqCst) {
                    app.reset_metrics();
                    reset_done_t.store(true, Ordering::SeqCst);
                }
                let now = elapsed_ms(origin);
                let _ = app.poll(now);
                if let Some(job) = notify_t.lock().expect("n").take() {
                    pending = Some(job);
                }
                if let Some((path, payload)) = pending.take() {
                    let segs: Vec<&str> = path.iter().map(String::as_str).collect();
                    let body: &'static [u8] = if payload == site::OBS_BODY_2 {
                        site::OBS_BODY_2
                    } else if payload == site::OBS_BODY {
                        site::OBS_BODY
                    } else {
                        site::OBS_BODY_2
                    };
                    if let Ok(0) = app.notify(
                        now,
                        &segs,
                        Response::content(body).content_format(ContentFormat::TEXT_PLAIN),
                    ) {
                        pending = Some((path, payload));
                    }
                }
                let occupancy = occupancy_line(app.engine_mut());
                let metrics = app.metrics();
                *snap_t.lock().expect("snap") = ServerSnap {
                    occupancy,
                    metrics,
                    now_ms: now,
                };
                thread::yield_now();
            }
        })
        .map_err(|e| e.to_string())?;
    thread::sleep(Duration::from_millis(5));
    Ok(CoapticServer {
        addr,
        stop,
        reset,
        reset_done,
        notify,
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

    fn stats(&self) -> SeriesStats {
        let wall = summarize(&self.wall_us);
        let engine = if self.engine_ms.is_empty() {
            None
        } else {
            Some(summarize(&self.engine_ms))
        };
        SeriesStats {
            label: self.label,
            n: self.wall_us.len(),
            wall_ms: WallStats::from_us(&wall),
            engine_delta_ms: engine.as_ref().map(EngineStats::from_ms),
        }
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

    fn series(&self) -> Vec<SeriesStats> {
        [
            &self.get,
            &self.put,
            &self.post,
            &self.obs,
            &self.block2,
            &self.block1,
            &self.loop_,
        ]
        .into_iter()
        .map(Series::stats)
        .collect()
    }
}

struct NotifyReport {
    register: Series,
    notify: Series,
    loop_: Series,
    client_occupancy: String,
    client_metrics: Metrics,
    client_now_ms: u64,
    collected: usize,
}

impl NotifyReport {
    fn write(&self, indent: &str, out: &mut impl Write) -> io::Result<()> {
        self.register.write(indent, out)?;
        self.notify.write(indent, out)?;
        self.loop_.write(indent, out)
    }

    fn series(&self) -> Vec<SeriesStats> {
        [&self.register, &self.notify, &self.loop_]
            .into_iter()
            .map(Series::stats)
            .collect()
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

#[derive(Serialize)]
struct SeriesStats {
    label: &'static str,
    n: usize,
    wall_ms: WallStats,
    #[serde(skip_serializing_if = "Option::is_none")]
    engine_delta_ms: Option<EngineStats>,
}

#[derive(Serialize)]
struct WallStats {
    min: f64,
    mean: f64,
    p50: f64,
    p99: f64,
    max: f64,
}

impl WallStats {
    fn from_us(s: &Summary) -> Self {
        Self {
            min: us_ms(s.min),
            mean: s.mean / 1000.0,
            p50: us_ms(s.p50),
            p99: us_ms(s.p99),
            max: us_ms(s.max),
        }
    }
}

#[derive(Serialize)]
struct EngineStats {
    min: u64,
    mean: f64,
    max: u64,
}

impl EngineStats {
    fn from_ms(s: &Summary) -> Self {
        Self {
            min: s.min,
            mean: s.mean,
            max: s.max,
        }
    }
}

#[derive(Serialize)]
struct MetricsDto {
    rx_accepted: u32,
    rx_error: u32,
    tx_ok: u32,
    tx_fail: u32,
    con_retransmit: u32,
    give_up: u32,
    observe_notify: u32,
    observe_register: u32,
    observe_cancel: u32,
    block1_assemble: u32,
    block2_assemble: u32,
    progress: u32,
    saturated: u32,
    nstart_reject: u32,
    empty_ack: u32,
    empty_rst: u32,
}

impl From<Metrics> for MetricsDto {
    fn from(m: Metrics) -> Self {
        Self {
            rx_accepted: m.rx_accepted,
            rx_error: m.rx_error,
            tx_ok: m.tx_ok,
            tx_fail: m.tx_fail,
            con_retransmit: m.con_retransmit,
            give_up: m.give_up,
            observe_notify: m.observe_notify,
            observe_register: m.observe_register,
            observe_cancel: m.observe_cancel,
            block1_assemble: m.block1_assemble,
            block2_assemble: m.block2_assemble,
            progress: m.progress,
            saturated: m.saturated,
            nstart_reject: m.nstart_reject,
            empty_ack: m.empty_ack,
            empty_rst: m.empty_rst,
        }
    }
}

#[derive(Serialize)]
struct JsonPair {
    name: &'static str,
    series: Vec<SeriesStats>,
    occupancy: String,
    now_ms: u64,
    metrics: MetricsDto,
    #[serde(skip_serializing_if = "Option::is_none")]
    capacities: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    collected: Option<usize>,
}

#[derive(Serialize)]
struct JsonReport {
    schema: &'static str,
    caveat: &'static str,
    host: Option<String>,
    iterations: usize,
    timeout_ms: u64,
    block_timeout_ms: u64,
    wall_s: f64,
    observe_collected: usize,
    observe_notify: u32,
    pairs: Vec<JsonPair>,
}

#[allow(clippy::too_many_arguments)]
fn write_json_report(
    path: &std::path::Path,
    cfg: &Config,
    wall: Duration,
    rs_to_coaptic: &PairReport,
    rs_to_coaptic_snap: &ServerSnap,
    coaptic_to_rs: &PairReport,
    client_occ: &str,
    client_metrics: &Metrics,
    client_now: u64,
    caps: &str,
    notify: &NotifyReport,
    notify_server: &ServerSnap,
) -> Result<(), PeerError> {
    let host = std::env::var("HOST")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok();
    let report = JsonReport {
        schema: "coaptic-dogfood/1",
        caveat: "Timings and counters are host/load specific (example host). Not a CI golden.",
        host,
        iterations: cfg.iterations,
        timeout_ms: u64::try_from(cfg.timeout.as_millis()).unwrap_or(u64::MAX),
        block_timeout_ms: u64::try_from(cfg.block_timeout.as_millis()).unwrap_or(u64::MAX),
        wall_s: wall.as_secs_f64(),
        observe_collected: notify.collected,
        observe_notify: notify_server.metrics.observe_notify,
        pairs: vec![
            JsonPair {
                name: "coap-rs → coaptic",
                series: rs_to_coaptic.series(),
                occupancy: rs_to_coaptic_snap.occupancy.clone(),
                now_ms: rs_to_coaptic_snap.now_ms,
                metrics: MetricsDto::from(rs_to_coaptic_snap.metrics),
                capacities: None,
                collected: None,
            },
            JsonPair {
                name: "coaptic → coap-rs",
                series: coaptic_to_rs.series(),
                occupancy: client_occ.to_owned(),
                now_ms: client_now,
                metrics: MetricsDto::from(*client_metrics),
                capacities: Some(caps.to_owned()),
                collected: None,
            },
            JsonPair {
                name: "coaptic ↔ coaptic observe notify",
                series: notify.series(),
                occupancy: notify_server.occupancy.clone(),
                now_ms: notify_server.now_ms,
                metrics: MetricsDto::from(notify_server.metrics),
                capacities: None,
                collected: Some(notify.collected),
            },
        ],
    };
    let file = std::fs::File::create(path).map_err(|e| format!("json create {path:?}: {e}"))?;
    serde_json::to_writer_pretty(file, &report).map_err(|e| format!("json write: {e}"))?;
    Ok(())
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
        // Fresh UDP client per send_request: Observe=1 cannot match the
        // registration Token. DELETE /obs drops the resource's observers
        // so high-N does not fill the 4-row table.
        let mut del = ClientRequest::request(Code::DELETE, &["obs"]);
        del.timeout = cfg.timeout;
        let got = client.send_request(dest, &del)?;
        expect_codes("DELETE /obs", i, got.code, &[Code::DELETED])?;
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
) -> Result<(PairReport, String, Metrics, u64, String), PeerError> {
    let (sock, _) = bind_loopback().map_err(|e| e.to_string())?;
    let mut app = App::profile::<profiles::Default>()
        .block_wise::<true>()
        .bind(sock)
        .map_err(|e| format!("bind: {e}"))?;
    app.reset_metrics();
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
    let occupancy = occupancy_line(app.engine_mut());
    let counters = app.metrics();
    Ok((report, occupancy, counters, now, caps))
}

fn run_coaptic_observe_notify(
    cfg: &Config,
    dest: SocketAddr,
    server: &CoapticServer,
) -> Result<NotifyReport, PeerError> {
    let (sock, _) = bind_loopback().map_err(|e| e.to_string())?;
    let mut app = App::profile::<profiles::Default>()
        .block_wise::<true>()
        .bind(sock)
        .map_err(|e| format!("bind: {e}"))?;
    app.reset_metrics();
    let origin = Instant::now();
    let peer = Endpoint::from(dest);
    let mut register = Series::new("OBS register /obs");
    let mut notify = Series::new("OBS notify /obs");
    let mut loop_ = Series::new("OBS collect (reg+notify)");
    let mut collected = 0usize;

    for i in 0..cfg.iterations {
        let loop_t0 = Instant::now();
        let loop_e0 = elapsed_ms(origin);

        let e0 = elapsed_ms(origin);
        let t0 = Instant::now();
        let now = e0.saturating_add(1);
        let call = app
            .get("obs")
            .observe()
            .to(peer)
            .send(now)
            .map_err(|e| format!("send OBS GET /obs: {e}"))?;
        let initial = wait_call(&mut app, call, origin, cfg.timeout)?;
        let e1 = elapsed_ms(origin);
        register.record(t0.elapsed(), Some(e1.saturating_sub(e0)));
        expect_codes("OBS register /obs", i, initial.code, &[Code::CONTENT])?;
        if initial.observe.is_none() {
            return Err(PeerError(format!(
                "iteration {i} OBS register /obs: missing Observe"
            )));
        }

        let e0 = elapsed_ms(origin);
        let t0 = Instant::now();
        server.notify(&["obs"], site::OBS_BODY_2)?;
        let note = wait_call(&mut app, call, origin, cfg.timeout)?;
        let e1 = elapsed_ms(origin);
        notify.record(t0.elapsed(), Some(e1.saturating_sub(e0)));
        expect_codes("OBS notify /obs", i, note.code, &[Code::CONTENT])?;
        if note.observe.is_none() {
            return Err(PeerError(format!(
                "iteration {i} OBS notify /obs: missing Observe"
            )));
        }
        if note.payload != site::OBS_BODY_2 {
            return Err(PeerError(format!(
                "iteration {i} OBS notify /obs: payload {:?} expected {:?}",
                note.payload,
                site::OBS_BODY_2
            )));
        }
        collected += 1;

        let now = elapsed_ms(origin).saturating_add(1);
        let off = app
            .get("obs")
            .deregister()
            .to(peer)
            .send(now)
            .map_err(|e| format!("send OBS deregister: {e}"))?;
        wait_call(&mut app, off, origin, cfg.timeout)?;

        loop_.record(
            loop_t0.elapsed(),
            Some(elapsed_ms(origin).saturating_sub(loop_e0)),
        );
    }

    let now = elapsed_ms(origin);
    Ok(NotifyReport {
        register,
        notify,
        loop_,
        client_occupancy: occupancy_line(app.engine_mut()),
        client_metrics: app.metrics(),
        client_now_ms: now,
        collected,
    })
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
        let cfg = Config::from_args([
            "--iterations",
            "3",
            "--timeout-ms=200",
            "--json",
            "/tmp/dogfood.json",
        ])
        .unwrap();
        assert_eq!(cfg.iterations, 3);
        assert_eq!(cfg.timeout.as_millis(), 200);
        assert_eq!(
            cfg.json_path.as_deref(),
            Some(std::path::Path::new("/tmp/dogfood.json"))
        );
    }

    #[test]
    fn config_default_is_high_n() {
        let cfg = Config::default();
        assert!(cfg.iterations > 2, "default must be above CI smoke");
        assert_eq!(cfg.iterations, 50);
        assert!(cfg.json_path.is_none());
    }

    #[test]
    fn timed_dogfood_smoke() {
        let mut buf = Vec::new();
        let json_path =
            std::env::temp_dir().join(format!("coaptic-dogfood-smoke-{}.json", std::process::id()));
        // 5 > Default observe table (4) so a missing deregister fails the run.
        let cfg = Config {
            iterations: 5,
            json_path: Some(json_path.clone()),
            ..Config::default()
        };
        run(cfg, &mut buf).expect("dogfood smoke");
        let s = String::from_utf8_lossy(&buf);
        assert!(s.contains("coap-rs → coaptic"), "{s}");
        assert!(s.contains("coaptic → coap-rs"), "{s}");
        assert!(s.contains("coaptic ↔ coaptic  observe notify"), "{s}");
        assert!(s.contains("OBS notify /obs"), "{s}");
        assert!(s.contains("LOOP (all verbs)"), "{s}");
        assert!(s.contains("app.metrics()"), "{s}");
        assert!(s.contains("rx_accepted="), "{s}");
        assert!(s.contains("observe  collected=5"), "{s}");
        assert!(s.contains("dogfood  ok"), "{s}");
        let report: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&json_path).expect("json file")).expect("json");
        let _ = std::fs::remove_file(&json_path);
        assert_eq!(report["schema"], "coaptic-dogfood/1");
        assert_eq!(report["iterations"], 5);
        assert_eq!(report["observe_collected"], 5);
        assert!(
            report["observe_notify"].as_u64().unwrap_or(0) >= 5,
            "{report}"
        );
        eprintln!("{s}");
    }
}
