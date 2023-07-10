//! Forwarder example based on https://github.com/C-o-r-E/nng_rs_pubsub_forwarder
//!
//! This example shows how to use raw sockets to set up a forwarder or proxy for pub/sub.
//!
//! An example setup for running this example would involve the following:
//!
//!  - Run this example binary (in the background or a terminal, etc) `forwarder tcp://localhost:3327 tcp://localhost:3328`
//!  - In a new terminal, run `nngcat --sub --dial "tcp://localhost:3328" --quoted`
//!  - In a second terminal, run `nngcat --sub --dial "tcp://localhost:3328" --quoted`
//!  - In a third terminal, run `for n in $(seq 0 99); do nngcat --pub --dial "tcp://localhost:3327" --data "$n"; done`

use nng::{forwarder, Error, Protocol, RawSocket};
use std::{env, process};

fn main() -> Result<(), Error> {
    let args: Vec<_> = env::args().collect();

    if args.len() < 3 {
        println!("Usage: {} <FRONT_END_URL> <BACK_END_URL>", &args[0]);
        process::exit(1);
    }

    do_forwarding(&args[1], &args[2])
}

fn do_forwarding(front_url: &str, back_url: &str) -> Result<(), Error> {
    let front_end: RawSocket = RawSocket::new(Protocol::Sub0)?;
    let back_end: RawSocket = RawSocket::new(Protocol::Pub0)?;

    front_end.socket.listen(front_url)?;
    back_end.socket.listen(back_url)?;

    forwarder(front_end, back_end)
}