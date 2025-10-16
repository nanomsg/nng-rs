//! # Asynchronous Request/Reply with Contexts
//!
//! Demonstrates concurrent request handling using NNG contexts.
//! Based on the NNG demo at <https://github.com/nanomsg/nng/tree/main/demo/async>.
//!
//! The client sends delay requests (duration in milliseconds) and the server waits
//! for that duration before responding. Multiple requests are handled concurrently
//! using separate NNG contexts.
//!
//! ## Usage
//!
//! ```bash
//! # Terminal 1: Start server with 3 parallel contexts
//! cargo run --example async_reqrep server tcp://127.0.0.1:5556 --parallel 3
//!
//! # Terminal 2: Run client with 500ms delay requests
//! cargo run --example async_reqrep client 500 tcp://127.0.0.1:5556
//! ```

use anng::{Message, protocols::reqrep0};
use lexopt::prelude::*;
use std::ffi::CString;
use std::io::Write;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug)]
enum Mode {
    Client { delay_ms: u64 },
    Server { parallel: usize },
}

#[derive(Debug)]
struct Args {
    mode: Mode,
    url: String,
}

fn parse_args() -> Result<Args, Box<dyn std::error::Error + Send + Sync>> {
    let mut mode = None;
    let mut url = String::from("tcp://127.0.0.1:5556");
    let mut parallel = 5usize; // Default parallelism

    let mut parser = lexopt::Parser::from_env();
    while let Some(arg) = parser.next()? {
        match arg {
            Value(val) if mode.is_none() => {
                let mode_str = val.string()?;
                mode = Some(match mode_str.as_str() {
                    "client" => {
                        // Next argument should be delay in milliseconds
                        if let Some(Value(delay_val)) = parser.next()? {
                            let delay_ms = delay_val.string()?.parse()?;
                            Mode::Client { delay_ms }
                        } else {
                            return Err("client mode requires delay argument (milliseconds)".into());
                        }
                    }
                    "server" => Mode::Server { parallel },
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
            Long("parallel") => {
                parallel = parser.value()?.string()?.parse()?;
                // Update server mode if already set
                if let Some(Mode::Server { .. }) = mode {
                    mode = Some(Mode::Server { parallel });
                }
            }
            Long("help") => {
                println!("Usage: async_reqrep <client|server> [OPTIONS] [URL]");
                println!();
                println!("Demonstrates asynchronous Request/Reply with configurable concurrency.");
                println!();
                println!("Arguments:");
                println!("  client <DELAY_MS>   Run as client, send delay requests");
                println!("  server              Run as server, handle delay requests");
                println!("  [URL]              URL to connect to (default: tcp://127.0.0.1:5556)");
                println!();
                println!("Options:");
                println!(
                    "  --parallel <N>     Number of parallel contexts for server (default: 5)"
                );
                println!();
                println!("Examples:");
                println!("  async_reqrep server tcp://127.0.0.1:5556 --parallel 10");
                println!("  async_reqrep client tcp://127.0.0.1:5556 1500");
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
    // Initialize tracing for better debugging
    tracing_subscriber::fmt::init();

    let args = parse_args()?;

    match args.mode {
        Mode::Server { parallel } => run_server(&args.url, parallel).await,
        Mode::Client { delay_ms } => run_client(&args.url, delay_ms).await,
    }
}

/// Run the concurrent delay service server
///
/// The server spawns multiple worker tasks, each with its own context, to handle
/// requests concurrently. Each request contains a delay duration, and the server
/// waits for that duration before responding.
async fn run_server(
    url: &str,
    parallel: usize,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!(
        "SERVER: Starting async delay service on {} with {} parallel contexts",
        url, parallel
    );

    // Create REP socket and listen for connections
    let c_url = CString::new(url)?;
    let socket = Arc::new(reqrep0::Rep0::listen(&c_url).await?);

    // Spawn parallel worker tasks, each with its own context
    let mut tasks = Vec::new();
    for worker_id in 0..parallel {
        let socket = Arc::clone(&socket);
        let task = tokio::spawn(async move { server_worker(worker_id, socket).await });
        tasks.push(task);
    }

    println!("SERVER: Spawned {} worker tasks", parallel);

    // Wait for all workers (they run forever)
    for task in tasks {
        if let Err(e) = task.await {
            eprintln!("Worker task failed: {}", e);
        }
    }

    Ok(())
}

/// Individual server worker that handles requests using its own context
///
/// Each worker maintains its own NNG context, allowing multiple requests
/// to be processed concurrently without interfering with each other.
async fn server_worker(
    worker_id: usize,
    socket: Arc<anng::Socket<reqrep0::Rep0>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut ctx = socket.context();
    let mut request_count = 0u64;

    println!("WORKER {}: Started", worker_id);

    loop {
        // Wait for incoming request
        let (request, responder) = ctx.receive().await?;
        request_count += 1;

        println!("WORKER {}: Handling request #{}", worker_id, request_count);

        // Parse the delay request - expect text containing milliseconds
        let delay_str = std::str::from_utf8(request.as_slice())?;
        let delay_ms: u64 = delay_str
            .trim()
            .parse()
            .map_err(|_| format!("invalid delay value: '{}'", delay_str))?;

        println!(
            "WORKER {}: Request #{} asks for {}ms delay",
            worker_id, request_count, delay_ms
        );

        // Process the request asynchronously - this is where the concurrency shines
        let start_time = Instant::now();
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        let actual_delay = start_time.elapsed();

        println!(
            "WORKER {}: Completed request #{} after {:?}",
            worker_id, request_count, actual_delay
        );

        // Create response with actual delay time
        let response_text = format!("Delayed for {}ms (actual: {:?})", delay_ms, actual_delay);
        let mut reply = Message::with_capacity(response_text.len());
        reply.write_all(response_text.as_bytes())?;

        // Send the reply
        if let Err((_responder, error, _message)) = responder.reply(reply).await {
            println!("WORKER {}: Failed to send reply: {}", worker_id, error);
            // In production, could retry with responder.reply(message)
        }
    }
}

/// Run the delay service client
///
/// The client sends multiple delay requests to demonstrate the server's
/// concurrent processing capabilities. Multiple requests can be sent
/// simultaneously to show how they're handled in parallel.
async fn run_client(
    url: &str,
    delay_ms: u64,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!("CLIENT: Connecting to delay service at {}", url);
    println!("CLIENT: Will send requests for {}ms delays", delay_ms);

    // Create REQ socket and connect to server
    let c_url = CString::new(url)?;
    let socket = reqrep0::Req0::dial(&c_url).await?;

    // Send requests sequentially to demonstrate server concurrency
    let num_requests = 5;
    let mut ctx = socket.context();

    for i in 1..=num_requests {
        let request_delay = delay_ms + (i * 100); // Vary delays slightly

        println!("CLIENT: Sending request {} ({}ms delay)", i, request_delay);

        let start_time = Instant::now();

        // Create request with delay duration
        let request_text = request_delay.to_string();
        let mut request = Message::with_capacity(request_text.len());
        request.write_all(request_text.as_bytes())?;

        // Send request and await reply
        match ctx.request(request).await {
            Ok(reply_future) => match reply_future.await {
                Ok(reply) => {
                    let total_time = start_time.elapsed();
                    let response_text = std::str::from_utf8(reply.as_slice())?;
                    println!(
                        "CLIENT: Request {}: Received reply after {:?}: {}",
                        i, total_time, response_text
                    );
                }
                Err(error) => {
                    println!("CLIENT: Request {}: Failed to receive reply: {}", i, error);
                }
            },
            Err((error, _message)) => {
                println!("CLIENT: Request {}: Failed to send: {}", i, error);
            }
        }

        // Brief pause between requests to see server handling them concurrently
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    println!("CLIENT: All requests completed");
    Ok(())
}
