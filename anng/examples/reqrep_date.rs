//! # Request/Reply Date Service
//!
//! A simple date service demonstrating the Request/Reply pattern.
//! Based on the NNG demo at <https://github.com/nanomsg/nng/tree/main/demo/reqrep>.
//!
//! The client sends DATE commands (binary value 0x1) and the server responds with
//! UNIX timestamps. The server intentionally skips every 6th reply to show retry behavior.
//!
//! ## Usage
//!
//! ```bash
//! # Terminal 1: Start the server
//! cargo run --example reqrep_date server tcp://127.0.0.1:5555
//!
//! # Terminal 2: Run the client
//! cargo run --example reqrep_date client tcp://127.0.0.1:5555
//! ```

use anng::{protocols::reqrep0, Message};
use std::ffi::CString;
use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

/// Command constants for the binary protocol
const DATE_COMMAND: u64 = 0x1;

#[derive(Debug)]
enum Mode {
    Client,
    Server,
}

#[derive(Debug)]
struct Args {
    mode: Mode,
    url: String,
}

fn parse_args() -> Result<Args, Box<dyn std::error::Error + Send + Sync>> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        eprintln!("Usage: {} <client|server> [URL]", args[0]);
        eprintln!();
        eprintln!("A simple Request/Reply date service demonstrating NNG patterns.");
        eprintln!();
        eprintln!("Arguments:");
        eprintln!("  <client|server>  Whether to run as client or server");
        eprintln!("  [URL]           URL to connect to (default: tcp://127.0.0.1:5555)");
        std::process::exit(1);
    }

    let mode = match args[1].as_str() {
        "client" => Mode::Client,
        "server" => Mode::Server,
        other => {
            return Err(format!("expected 'client' or 'server', got '{}'", other).into());
        }
    };

    let url = args
        .get(2)
        .cloned()
        .unwrap_or_else(|| "tcp://127.0.0.1:5555".to_string());

    Ok(Args { mode, url })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Initialize tracing for better debugging
    tracing_subscriber::fmt::init();

    let args = parse_args()?;

    match args.mode {
        Mode::Server => run_server(&args.url).await,
        Mode::Client => run_client(&args.url).await,
    }
}

/// Run the date service server
///
/// The server listens for DATE commands (0x1) and responds with the current UNIX timestamp.
/// To demonstrate resilience, it intentionally skips every 6th reply.
async fn run_server(url: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!("SERVER: Starting date service on {}", url);

    // Create REP socket and listen for connections
    let c_url = CString::new(url)?;
    let socket = reqrep0::Rep0::listen(&c_url).await?;
    let mut ctx = socket.context();

    let mut request_count = 0u64;

    loop {
        // Wait for incoming request
        let (request, responder) = ctx.receive().await?;
        request_count += 1;

        // Parse the binary request - expect 8 bytes containing the command
        if request.len() != 8 {
            println!("SERVER: Invalid request length: {} bytes", request.len());
            continue;
        }

        let command_bytes: [u8; 8] = request.as_slice().try_into()?;
        let command = u64::from_be_bytes(command_bytes);

        match command {
            DATE_COMMAND => {
                println!("SERVER: Received DATE request #{}", request_count);

                // Resilience demonstration: skip every 6th reply to show retry behavior
                if request_count.is_multiple_of(6) {
                    println!(
                        "SERVER: Intentionally skipping reply #{} (demonstrating resilience)",
                        request_count
                    );
                    continue; // Drop the responder, client will retry
                }

                // Get current timestamp
                let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();

                println!("SERVER: Sending timestamp: {}", timestamp);

                // Create binary reply with timestamp in network byte order
                let mut reply = Message::with_capacity(8);
                reply.write_all(&timestamp.to_be_bytes())?;

                // Send the reply
                if let Err((_responder, error, _message)) = responder.reply(reply).await {
                    println!("SERVER: Failed to send reply: {}", error);
                    // In production, could retry with responder.reply(message)
                }
            }
            _ => {
                println!("SERVER: Unknown command: 0x{:x}", command);
                // Still need to reply to maintain protocol state
                let mut reply = Message::with_capacity(8);
                reply.write_all(&0u64.to_be_bytes())?; // Send 0 for unknown commands
                let _ = responder.reply(reply).await;
            }
        }
    }
}

/// Run the date service client
///
/// The client sends DATE commands in a loop and displays the server responses.
/// It demonstrates automatic retry behavior when the server doesn't respond.
async fn run_client(url: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!("CLIENT: Connecting to date service at {}", url);

    // Create REQ socket and connect to server
    let c_url = CString::new(url)?;
    let socket = reqrep0::Req0::dial(&c_url).await?;
    let mut ctx = socket.context();

    // Send requests in a loop
    for i in 1..=20 {
        println!("CLIENT: Sending DATE request #{}", i);

        // Create binary request with DATE command in network byte order
        let mut request = Message::with_capacity(8);
        request.write_all(&DATE_COMMAND.to_be_bytes())?;

        // Send request and await reply
        match ctx.request(request).await {
            Ok(reply_future) => {
                match reply_future.await {
                    Ok(reply) => {
                        // Parse binary reply - expect 8 bytes containing timestamp
                        if reply.len() == 8 {
                            let timestamp_bytes: [u8; 8] = reply.as_slice().try_into()?;
                            let timestamp = u64::from_be_bytes(timestamp_bytes);

                            if timestamp > 0 {
                                println!("CLIENT: Received timestamp: {}", timestamp);
                            } else {
                                println!("CLIENT: Server sent error response");
                            }
                        } else {
                            println!("CLIENT: Invalid reply length: {} bytes", reply.len());
                        }
                    }
                    Err(error) => {
                        println!(
                            "CLIENT: Request failed: {} (server may have skipped reply)",
                            error
                        );
                    }
                }
            }
            Err((error, _message)) => {
                println!("CLIENT: Failed to send request: {}", error);
                // In production, could retry with the returned message
            }
        }

        // Wait between requests
        tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
    }

    println!("CLIENT: Finished sending requests");
    Ok(())
}
