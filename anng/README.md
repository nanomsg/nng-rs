# anng

Safe, async Rust bindings for [NNG (nanomsg next-generation)](https://nng.nanomsg.org/).

## What is NNG?

From the [NNG GitHub repository](https://github.com/nanomsg/nng):

> NNG, like its predecessors [nanomsg](http://nanomsg.org) (and to some extent
> [ZeroMQ](http://zeromq.org/)), is a lightweight, broker-less library,
> offering a simple API to solve common recurring messaging problems,
> such as publish/subscribe, RPC-style request/reply, or service discovery.
> The API frees the programmer from worrying about details like connection
> management, retries, and other common considerations, so that they
> can focus on the application instead of the plumbing.

## Why anng?

- **Type-safe protocols**:
  Compile-time prevention of protocol violations using Rust's type system.
- **Async-first**:
  Native async/await integration with Tokio and cancellation safety.
- **Zero-cost abstractions**:
  Memory safety and protocol correctness with no runtime overhead over NNG itself.

All [NNG protocols](https://nng.nanomsg.org/ref/proto/index.html) are
supported: REQ/REP, PUB/SUB, PUSH/PULL, SURVEYOR/RESPONDENT, BUS, and
PAIR.

## Quick Start

Add to your `Cargo.toml`:

```toml
[dependencies]
anng = "0.1"
tokio = { version = "1", features = ["full"] }
```

### Request/Reply Example

```rust
use anng::{protocols::reqrep0, Message};
use std::io::Write;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Server
    tokio::spawn(async {
        let socket = reqrep0::Rep0::listen(c"tcp://127.0.0.1:8080").await?;
        let mut ctx = socket.context();

        let (request, responder) = ctx.receive().await?;
        println!("Request: {:?}", request.as_slice());

        let mut reply = Message::with_capacity(100);
        write!(&mut reply, "Hello back!")?;
        responder.reply(reply).await?;

        Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    });

    // Client
    let socket = reqrep0::Req0::dial(c"tcp://127.0.0.1:8080").await?;
    let mut ctx = socket.context();

    let mut request = Message::with_capacity(100);
    write!(&mut request, "Hello server!")?;

    let reply = ctx.request(request).await?.await?;
    println!("Reply: {:?}", reply.as_slice());

    Ok(())
}
```

## Type Safety

The library uses phantom types to enforce protocol correctness at compile time:

```rust
// ✅ This compiles - correct protocol usage
let req_socket = reqrep0::Req0::dial(url).await?;
let reply = req_socket.context().request(msg).await?;

// ❌ This won't compile - protocol violation
let req_socket = reqrep0::Req0::dial(url).await?;
req_socket.publish(msg).await?; // Error: method `publish` not found
```

Socket types like `Socket<reqrep0::Req0>` only expose methods valid for
that protocol, making it impossible to accidentally call `publish()` on
a request socket or `reply()` before receiving a request.

## Concurrency and Contexts

For concurrent operations, create contexts from sockets:

```rust
use std::sync::Arc;

let socket = Arc::new(reqrep0::Rep0::listen(url).await?);

// Spawn multiple workers for concurrent request handling
for worker_id in 0..4 {
    let socket = Arc::clone(&socket);
    tokio::spawn(async move {
        let mut ctx = socket.context();
        loop {
            let (request, responder) = ctx.receive().await?;
            // Handle request concurrently...
            responder.reply(process_request(request)).await?;
        }
    });
}
```

Each context maintains independent protocol state, enabling safe
concurrent operations on the same socket.

## Cancellation Safety

All async operations are cancellation safe - futures can be dropped at
any time without corrupting the NNG state. Cancelling a send may drop
the message if dropped before NNG effects the send. Cancelling a receive
will not drop an already-received message (in the case of a race). The
library automatically handles NNG cleanup and message recovery when
operations are cancelled.

## Learning More

- **Protocol concepts**: See the [NNG protocol documentation](https://nng.nanomsg.org/ref/proto/index.html) to understand when to use each messaging pattern
- **API reference**: Full documentation available on [docs.rs](https://docs.rs/anng)
- **NNG manual**: [https://nng.nanomsg.org/ref/preface.html](https://nng.nanomsg.org/ref/preface.html)

## License

Licensed under either of

* Apache License, Version 2.0
   ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
* MIT license
   ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
