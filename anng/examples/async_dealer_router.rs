//! # Async Dealer-Router Pattern
//!
//! Demonstrates high-performance asynchronous request handling using Raw REP (Router)
//! and Raw REQ (Dealer) sockets.
//!
//! - **Router (Server)**: Listens for connections, receives requests asynchronously,
//!   and sends replies when processing is complete. It does not enforce a strict
//!   Request-Reply sequence, enabling "infinite receive" and concurrent processing.
//!
//! - **Dealer (Client)**: Connects to the server, sends requests asynchronously
//!   without waiting for immediate replies, and then processes replies as they arrive.
//!   This allows high throughput and pipelining of requests.
//!
//! This pattern is ideal for load balancers, proxies, and high-concurrency servers.

use anng::{Message, protocols::reqrep0::Rep0Raw, protocols::reqrep0::Req0Raw};
use lexopt::prelude::*;
use std::ffi::CString;
use std::io::Write; // For Message write
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

/// A helper wrapper for Raw REQ (Dealer) socket.
///
/// Handles the manual management of Request ID headers required by Raw REQ sockets.
/// This allows "fire and forget" asynchronous requests that are compatible with Router sockets.
pub struct AsyncDealer {
    socket: anng::Socket<Req0Raw>,
    next_request_id: AtomicU32,
}

impl AsyncDealer {
    /// Connects to the specified URL.
    pub async fn dial(url: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let c_url = CString::new(url)?;
        let socket = Req0Raw::dial(&c_url).await?;
        Ok(Self {
            socket,
            // Start with high bit set to avoid conflict with user IDs if any,
            // though for Dealer it's mostly arbitrary as long as it's unique per connection.
            next_request_id: AtomicU32::new(0x80000000),
        })
    }

    /// Sends a request asynchronously.
    ///
    /// Automatically appends a Request ID to the header.
    /// Returns the Request ID used for this message.
    pub async fn send_request(
        &mut self,
        body: &[u8],
    ) -> Result<u32, Box<dyn std::error::Error + Send + Sync>> {
        let req_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);

        let mut msg = Message::with_capacity(body.len());
        msg.write_all(body)?;

        // Append Request ID to header (Big Endian)
        msg.write_header(&req_id.to_be_bytes())?;

        self.socket.send(msg).await.map_err(|(e, _)| e)?;
        Ok(req_id)
    }

    /// Receives a reply.
    ///
    /// Returns the reply message. The message body contains the response.
    /// The header (if any) is preserved but usually not needed for processing.
    pub async fn recv_reply(
        &mut self,
    ) -> Result<Message, Box<dyn std::error::Error + Send + Sync>> {
        let msg = self.socket.recv().await?;
        // Note: We could strip the header here if we wanted to enforce a "clean" body,
        // but often the header is empty on receive for Dealer (request ID is stripped by NNG usually).
        Ok(msg)
    }
}

/// A helper wrapper for Raw REP (Router) socket.
///
/// Simplifies the usage of Router sockets for asynchronous server patterns.
pub struct AsyncRouter {
    socket: anng::Socket<Rep0Raw>,
}

impl AsyncRouter {
    /// Listens on the specified URL.
    pub async fn listen(url: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let c_url = CString::new(url)?;
        let socket = Rep0Raw::listen(&c_url).await?;
        Ok(Self { socket })
    }

    /// Receives a request.
    ///
    /// Returns a `RoutedMessage` which contains the original message with its routing header preserved.
    /// This message MUST be used to send the reply via `send_reply` to ensure correct routing.
    pub async fn recv_request(
        &mut self,
    ) -> Result<Message, Box<dyn std::error::Error + Send + Sync>> {
        let msg = self.socket.recv().await?;
        Ok(msg)
    }

    /// Sends a reply to a previously received request.
    ///
    /// Takes the `msg` that was received (which contains the routing header)
    /// and sends it back. The body of the message should have been updated with the response.
    pub async fn send_reply(
        &mut self,
        msg: Message,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.socket.send(msg).await.map_err(|(e, _)| e)?;
        Ok(())
    }

    /// Clones the router to allow concurrent sending/receiving.
    ///
    /// Each clone shares the underlying NNG socket but has its own asynchronous context (Aio).
    pub fn clone(&self) -> Self {
        Self {
            socket: self.socket.clone(),
        }
    }
}

#[derive(Debug)]
enum Mode {
    Client { delay_ms: u64 },
    Server,
}

#[derive(Debug)]
struct Args {
    mode: Mode,
    url: String,
}

fn parse_args() -> Result<Args, Box<dyn std::error::Error + Send + Sync>> {
    let mut mode = None;
    let mut url = String::from("tcp://127.0.0.1:5560");

    let mut parser = lexopt::Parser::from_env();
    while let Some(arg) = parser.next()? {
        match arg {
            Value(val) if mode.is_none() => {
                let mode_str = val.string()?;
                mode = Some(match mode_str.as_str() {
                    "client" => {
                        if let Some(Value(delay_val)) = parser.next()? {
                            let delay_ms = delay_val.string()?.parse()?;
                            Mode::Client { delay_ms }
                        } else {
                            return Err("client mode requires delay argument (milliseconds)".into());
                        }
                    }
                    "server" => Mode::Server,
                    other => {
                        return Err(
                            format!("expected 'client' or 'server', got '{}'", other).into()
                        );
                    }
                });
            }
            Value(val) => {
                url = val.string()?;
            }
            Long("help") => {
                println!("Usage: async_dealer_router <client|server> [OPTIONS] [URL]");
                std::process::exit(0);
            }
            _ => return Err(arg.unexpected().into()),
        }
    }

    let mode = mode.ok_or("missing required argument: <client|server>")?;
    Ok(Args { mode, url })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt::init();
    let args = parse_args()?;

    match args.mode {
        Mode::Server => run_server(&args.url).await,
        Mode::Client { delay_ms } => run_client(&args.url, delay_ms).await,
    }
}

async fn run_server(url: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!("SERVER: Starting async router on {}", url);

    // Create and listen with a Raw REP socket
    let socket = AsyncRouter::listen(url).await?;

    // We can clone the socket! The inner handle is shared, but each clone gets its own Aio.
    // This allows us to have a dedicated "receiver" loop and multiple "sender" tasks.
    let mut rx_socket = socket.clone();

    println!("SERVER: Ready to accept requests.");

    loop {
        // Receive request
        // In Raw mode, the message header contains the routing ID of the client.
        // We MUST preserve this header to reply to the correct client.
        let msg = match rx_socket.recv_request().await {
            Ok(msg) => msg,
            Err(e) => {
                eprintln!("Error receiving: {:?}", e);
                // Sleep a bit on error to avoid tight loop if something is broken
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };

        // Clone the socket for the worker task to send the reply.
        // Cloning is cheap (Arc increment + Aio allocation).
        let mut tx_socket = socket.clone();

        // Spawn a task to handle the request asynchronously
        tokio::spawn(async move {
            handle_request(msg, &mut tx_socket).await;
        });
    }
}

async fn run_client(
    url: &str,
    delay_ms: u64,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!("CLIENT: Connecting to delay service at {}", url);
    println!("CLIENT: Will send requests for {}ms delays", delay_ms);

    // Use AsyncDealer helper
    let mut dealer = AsyncDealer::dial(url).await?;

    // Send requests sequentially to demonstrate server concurrency
    let num_requests = 5;

    for i in 1..=num_requests {
        let request_delay = delay_ms + (i * 100); // Vary delays slightly
        println!("CLIENT: Sending request {} ({}ms delay)", i, request_delay);

        // Create request with delay duration
        let request_text = request_delay.to_string();

        // Send request asynchronously using helper
        dealer.send_request(request_text.as_bytes()).await?;

        // Brief pause between requests to see server handling them concurrently
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    println!("CLIENT: All requests sent, waiting for replies...");

    // Receive replies
    for _ in 1..=num_requests {
        let reply = dealer.recv_reply().await?;
        let response_text = String::from_utf8_lossy(reply.as_slice());
        println!("CLIENT: Received reply: {}", response_text);
    }

    println!("CLIENT: All requests completed");
    Ok(())
}

async fn handle_request(mut msg: Message, socket: &mut AsyncRouter) {
    // Parse request
    let req_str = String::from_utf8_lossy(msg.as_slice()).to_string();
    let delay_ms: u64 = req_str.trim().parse().unwrap_or(100);

    println!("Received request: {} (delay {}ms)", req_str, delay_ms);
    let start = Instant::now();

    // Simulate work (non-blocking sleep)
    // This is where the "async" magic happens. While we sleep here,
    // the main loop is already receiving the next request!
    tokio::time::sleep(Duration::from_millis(delay_ms)).await;

    // Prepare reply
    // We reuse the message to keep the routing header!
    // Just clear the body and write the response.
    msg.truncate(0);
    let response = format!("Processed {} in {:?}", req_str, start.elapsed());
    if let Err(e) = write!(&mut msg, "{}", response) {
        eprintln!("Failed to write response: {}", e);
        return;
    }

    // Send reply
    if let Err(e) = socket.send_reply(msg).await {
        eprintln!("Failed to send reply: {:?}", e);
    } else {
        println!("Sent reply: {}", response);
    }
}
