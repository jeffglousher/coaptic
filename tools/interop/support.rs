//! Small process protocol. stdout is JSON Lines; diagnostics use stderr.
use std::time::Duration;
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
    pub ipv6: bool,
    pub dtls: bool,
    pub port: u16,
    pub key: String,
    pub path: String,
    pub method: u8,
    pub payload: Vec<u8>,
    pub timeout: u64,
}
impl Args {
    pub fn parse() -> Result<Self, Error> {
        let a: Vec<_> = std::env::args().skip(1).collect();
        if !(7..=9).contains(&a.len()) {
            return Err(
                "usage: PEER server|client udp|dtls PORT KEY PATH METHOD TIMEOUT_MS [ipv4|ipv6] [PAYLOAD_HEX]"
                    .into(),
            );
        }
        if !matches!(a[0].as_str(), "server" | "client") || !matches!(a[1].as_str(), "udp" | "dtls")
        {
            return Err("invalid role, transport or method".into());
        }
        if !matches!(
            a[4].as_str(),
            "test" | "large" | "counter" | "missing" | "methods"
        ) {
            return Err("unsupported fixture path".into());
        }
        let method = match a[5].as_str() {
            "GET" => 1,
            "POST" => 2,
            "PUT" => 3,
            "DELETE" => 4,
            "FETCH" => 5,
            "PATCH" => 6,
            "IPATCH" => 7,
            _ => return Err("invalid method".into()),
        };
        let hex = a.get(8).map(String::as_str).unwrap_or("");
        if hex.len() > 512 || hex.len() % 2 != 0 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("invalid bounded payload hex".into());
        }
        let payload = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("checked hex"))
            .collect();
        let family = a.get(7).map(String::as_str).unwrap_or("ipv4");
        if !matches!(family, "ipv4" | "ipv6") {
            return Err("invalid address family".into());
        }
        let port = a[2].parse()?;
        if port == 0 {
            return Err("port must be nonzero".into());
        }
        if a[3].is_empty() || a[3].len() > 64 {
            return Err("PSK must be 1..64 bytes".into());
        }
        let timeout = a[6].parse()?;
        if !(100..=30000).contains(&timeout) {
            return Err("timeout must be 100..30000 ms".into());
        }
        Ok(Self {
            server: a[0] == "server",
            ipv6: family == "ipv6",
            dtls: a[1] == "dtls",
            port,
            key: a[3].clone(),
            path: a[4].clone(),
            method,
            payload,
            timeout,
        })
    }
    pub fn address(&self) -> std::net::SocketAddr {
        if self.ipv6 {
            (std::net::Ipv6Addr::LOCALHOST, self.port).into()
        } else {
            ([127, 0, 0, 1], self.port).into()
        }
    }
}
pub fn ready(peer: &str, stack: &str, port: u16, dtls: bool) {
    println!(
        "{}",
        serde_json::json!({"schema":"coaptic-peer/2","event":"ready","peer":peer,"stack":stack,"port":port,"transport":if dtls {"dtls"} else {"udp"}})
    );
}
pub fn response(code: u8, body: &[u8], elapsed: Duration) {
    let elapsed_ns = elapsed.as_nanos();
    let hex: String = body.iter().map(|b| format!("{b:02x}")).collect();
    println!(
        "{}",
        serde_json::json!({"schema":"coaptic-peer/2","event":"response","code":code,"payload_hex":hex,"elapsed_ns":elapsed_ns,"elapsed_us":elapsed_ns as f64 / 1000.0,"clock":{"name":"std::time::Duration","resolution_ns":null}})
    );
}
pub fn finish(result: Result<(), Error>) -> std::process::ExitCode {
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            println!(
                "{}",
                serde_json::json!({"schema":"coaptic-peer/2","event":"error","message":e.to_string()})
            );
            std::process::ExitCode::FAILURE
        }
    }
}

/// Application fixture, not a protocol implementation. Independent codecs
/// supply the method, bytes and format. C implements the same contract separately.
#[derive(Default)]
pub struct MethodResource(Option<Vec<u8>>);
impl MethodResource {
    pub const fn new() -> Self {
        Self(None)
    }
    pub fn respond(&mut self, method: u8, payload: &[u8], format_ok: bool) -> (u8, Vec<u8>) {
        if matches!(method, 2 | 3 | 5 | 6 | 7) && !format_ok {
            return (143, vec![]);
        }
        if payload.len() > 64 {
            return (141, vec![]);
        }
        match method {
            1 => self
                .0
                .as_ref()
                .map(|value| (69, value.clone()))
                .unwrap_or((132, vec![])),
            3 => {
                let code = if self.0.is_some() { 68 } else { 65 };
                self.0 = Some(payload.to_vec());
                (code, vec![])
            }
            4 => {
                if self.0.take().is_some() {
                    (66, vec![])
                } else {
                    (132, vec![])
                }
            }
            5 if payload != b"value" => (128, vec![]),
            5 => self
                .0
                .as_ref()
                .map(|value| (69, value.clone()))
                .unwrap_or((132, vec![])),
            2 | 6 | 7 => {
                let Some(value) = self.0.as_mut() else {
                    return (132, vec![]);
                };
                if method == 7 {
                    let Some(body) = payload.strip_prefix(b"=") else {
                        return (128, vec![]);
                    };
                    *value = body.to_vec();
                } else {
                    let body = if method == 6 {
                        let Some(body) = payload.strip_prefix(b"+") else {
                            return (128, vec![]);
                        };
                        body
                    } else {
                        payload
                    };
                    if value.len() + body.len() > 64 {
                        return (141, vec![]);
                    }
                    value.extend_from_slice(body);
                }
                (68, vec![])
            }
            _ => (133, vec![]),
        }
    }
}
