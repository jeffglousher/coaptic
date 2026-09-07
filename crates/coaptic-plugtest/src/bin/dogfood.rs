//! Timed coaptic ↔ coap-rs dogfood.
//!
//! ```text
//! cargo run -p coaptic-plugtest --bin dogfood
//! cargo run -p coaptic-plugtest --bin dogfood -- --iterations 2
//! ```

use std::env;
use std::io::{self, Write};
use std::process::ExitCode;

use coaptic_plugtest::dogfood::{self, Config};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{}", Config::USAGE);
        return ExitCode::SUCCESS;
    }
    let cfg = match Config::from_args(&args) {
        Ok(cfg) => cfg,
        Err(e) => {
            let _ = writeln!(io::stderr(), "{e}");
            return ExitCode::from(2);
        }
    };
    match dogfood::run(cfg, io::stdout()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            let _ = writeln!(io::stderr(), "dogfood: {e}");
            ExitCode::from(1)
        }
    }
}
