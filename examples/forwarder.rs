//! Forwarder example based on https://github.com/C-o-r-E/nng_rs_pubsub_forwarder
//!
//! This example shows how to use raw sockets to set up a forwarder or proxy for pub/sub.
//! 
//! An example setup for running this example would involve the following:
//! 
//!  - Run this example binary (in the background or a terminal, etc)
//!  - In a new terminal, run `nngcat --sub --dial "tcp://localhost:3328" --quoted`
//!  - In a second terminal, run `nngcat --sub --dial "tcp://localhost:3328" --quoted`
//!  - In a third terminal, run `for n in $(seq 0 99); do nngcat --pub --dial "tcp://localhost:3327" --data "$n"; done`

use nng::{
    RawSocket, Protocol, forwarder,
};

const FRONT_URL: &str = "tcp://localhost:3327";
const BACK_URL: &str = "tcp://localhost:3328";

fn main() {
    let front_end: RawSocket = RawSocket::new(Protocol::Sub0).unwrap();
    let back_end:  RawSocket = RawSocket::new(Protocol::Pub0).unwrap();

    let ret = front_end.socket.listen(FRONT_URL);
    if let Err(x) = ret { panic!("failed to listen on front end: {}", x) }

    let ret = back_end.socket.listen(BACK_URL);
    if let Err(x) = ret { panic!("failed to listen on back end: {}", x) }

    let ret = forwarder(
        front_end,
        back_end
    );

    if let Err(fw_err) = ret { println!("Forwarder exited with err: {}", fw_err) }
}