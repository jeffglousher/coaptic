//! Small process protocol. stdout is JSON Lines; diagnostics use stderr.
use std::time::Instant;
pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub const BODY: &[u8] = b"core-test-payload";
pub const LARGE: [u8; 2000] = {
    let mut bytes = [0; 2000];
    let mut i = 0;
    while i < bytes.len() {
        bytes[i] = (i % 251) as u8;
        i += 1;
    }
    bytes
};
pub struct Args {
    pub server: bool,
    pub dtls: bool,
    pub port: u16,
    pub key: String,
    pub path: String,
    pub post: bool,
    pub timeout: u64,
}
impl Args {
    pub fn parse() -> Result<Self, Error> {
        let a: Vec<_> = std::env::args().skip(1).collect();
        if a.len() != 7 {
            return Err(
                "usage: PEER server|client udp|dtls PORT KEY PATH GET|POST TIMEOUT_MS".into(),
            );
        }
        if !matches!(a[0].as_str(), "server" | "client")
            || !matches!(a[1].as_str(), "udp" | "dtls")
            || !matches!(a[5].as_str(), "GET" | "POST")
        {
            return Err("invalid role, transport or method".into());
        }
        if !matches!(a[4].as_str(), "test" | "large" | "counter" | "missing") {
            return Err("unsupported fixture path".into());
        }
        let timeout = a[6].parse()?;
        if !(100..=30000).contains(&timeout) {
            return Err("timeout must be 100..30000 ms".into());
        }
        Ok(Self {
            server: a[0] == "server",
            dtls: a[1] == "dtls",
            port: a[2].parse()?,
            key: a[3].clone(),
            path: a[4].clone(),
            post: a[5] == "POST",
            timeout,
        })
    }
    pub fn address(&self) -> std::net::SocketAddr {
        ([127, 0, 0, 1], self.port).into()
    }
}
pub fn ready(peer: &str, stack: &str, port: u16, dtls: bool) {
    println!(
        "{}",
        serde_json::json!({"schema":"coaptic-peer/1","event":"ready","peer":peer,"stack":stack,"port":port,"transport":if dtls {"dtls"} else {"udp"}})
    );
}
pub fn response(code: u8, body: &[u8], start: Instant) {
    let hex: String = body.iter().map(|b| format!("{b:02x}")).collect();
    println!(
        "{}",
        serde_json::json!({"schema":"coaptic-peer/1","event":"response","code":code,"payload_hex":hex,"elapsed_us":start.elapsed().as_micros()})
    );
}
pub fn finish(result: Result<(), Error>) -> std::process::ExitCode {
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            println!(
                "{}",
                serde_json::json!({"schema":"coaptic-peer/1","event":"error","message":e.to_string()})
            );
            std::process::ExitCode::FAILURE
        }
    }
}
