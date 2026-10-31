//! ssh tarpit plus optional reporting sinks in one process.
//! threads: accept -> client loop (tarpit) -> worker (filter, dedupe, sinks, event log)
#![deny(unsafe_code)]

#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        // not println!: it panics on a closed stdout, and a panic aborts the process
        let _ = writeln!(std::io::stdout(), "tarpit: {}", format_args!($($arg)*));
    }};
}

pub mod abuseipdb;
mod accept;
pub mod banner;
pub mod config;
mod dates;
pub mod events;
pub mod ip;
pub mod listen;
pub mod sinks;
mod slots;
pub mod tarpit;
mod throttle;
pub mod webhook;
pub mod worker;
