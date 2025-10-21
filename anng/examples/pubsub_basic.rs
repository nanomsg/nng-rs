//! # Basic Publish/Subscribe
//!
//! Demonstrates pub/sub messaging with topic filtering.
//! Inspired by the NNG demo at <https://github.com/nanomsg/nng/tree/main/demo/pubsub_forwarder>.
//!
//! The publisher sends messages with topic prefixes (news.sports, weather.forecast, etc.)
//! and subscribers receive only messages matching their topic subscriptions.
//!
//! ## Usage
//!
//! ```bash
//! # Terminal 1: Subscribe to news topics
//! cargo run --example pubsub_basic subscriber tcp://127.0.0.1:5557 news.
//!
//! # Terminal 2: Start publisher
//! cargo run --example pubsub_basic publisher tcp://127.0.0.1:5557
//!
//! # Terminal 3: Subscribe to all messages
//! cargo run --example pubsub_basic subscriber tcp://127.0.0.1:5557 ""
//! ```

use anng::{Message, protocols::pubsub0};
use std::ffi::CString;
use std::io::Write;

#[derive(Debug)]
enum Mode {
    Publisher,
    Subscriber { topics: Vec<String> },
}

#[derive(Debug)]
struct Args {
    mode: Mode,
    url: String,
}

fn parse_args() -> Result<Args, Box<dyn std::error::Error + Send + Sync>> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        eprintln!(
            "Usage: {} <publisher|subscriber> [URL] [TOPICS...]",
            args[0]
        );
        eprintln!();
        eprintln!("Demonstrates basic Publish/Subscribe with topic filtering.");
        eprintln!();
        eprintln!("Arguments:");
        eprintln!("  publisher           Run as publisher, send messages with various topics");
        eprintln!("  subscriber          Run as subscriber, receive filtered messages");
        eprintln!("  [URL]              URL to connect to (default: tcp://127.0.0.1:5557)");
        eprintln!("  [TOPICS...]        Topic prefixes to subscribe to (subscriber only)");
        eprintln!("                     Use empty string \"\" to receive all messages");
        eprintln!();
        eprintln!("Examples:");
        eprintln!("  {} publisher", args[0]);
        eprintln!("  {} subscriber news", args[0]);
        eprintln!("  {} subscriber \"\" # All messages", args[0]);
        std::process::exit(1);
    }

    let mode = match args[1].as_str() {
        "publisher" | "pub" => Mode::Publisher,
        "subscriber" | "sub" => {
            // Collect topic filters from remaining arguments (skipping URL if present)
            let mut topics = Vec::new();
            for arg in &args[2..] {
                if !arg.starts_with("tcp://") && !arg.starts_with("ipc://") {
                    topics.push(arg.clone());
                }
            }

            // Default to subscribing to all messages if no topics specified
            if topics.is_empty() {
                topics.push(String::new()); // Empty string = all messages
            }

            Mode::Subscriber { topics }
        }
        other => {
            return Err(format!("expected 'publisher' or 'subscriber', got '{}'", other).into());
        }
    };

    let url = args
        .iter()
        .skip(2)
        .find(|s| s.starts_with("tcp://") || s.starts_with("ipc://"))
        .cloned()
        .unwrap_or_else(|| "tcp://127.0.0.1:5557".to_string());

    Ok(Args { mode, url })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Initialize tracing for better debugging
    tracing_subscriber::fmt::init();

    let args = parse_args()?;

    match args.mode {
        Mode::Publisher => run_publisher(&args.url).await,
        Mode::Subscriber { topics } => run_subscriber(&args.url, &topics).await,
    }
}

/// Run the message publisher
///
/// The publisher sends messages with various topic prefixes to demonstrate
/// topic-based filtering. It publishes news, weather, and alert messages
/// with timestamps.
async fn run_publisher(url: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!("PUBLISHER: Starting on {}", url);

    // Create PUB socket and listen for subscriber connections
    let c_url = CString::new(url)?;
    let mut socket = pubsub0::Pub0::listen(&c_url).await?;

    // Wait a moment for subscribers to connect
    println!("PUBLISHER: Waiting for subscribers to connect...");
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // Define topics and sample messages
    let topics = [
        (
            "news",
            vec![
                "Local team wins championship!",
                "Storm warning issued",
                "Election results announced",
            ],
        ),
        (
            "weather",
            vec![
                "Sunny skies expected today",
                "Weekend rain expected",
                "Temperature dropping tonight",
            ],
        ),
        (
            "alerts",
            vec![
                "Emergency broadcast test",
                "Road closure notification",
                "System maintenance alert",
            ],
        ),
    ];

    let mut message_count = 0;

    // Publish messages in a loop to keep subscribers receiving data
    loop {
        for (topic, messages) in &topics {
            for message_text in messages {
                message_count += 1;

                // Create message with topic prefix and content
                let full_message = format!("{}: {}", topic, message_text);
                let mut message = Message::with_capacity(full_message.len());
                message.write_all(full_message.as_bytes())?;

                println!("PUBLISHER: Sending #{}: {}", message_count, full_message);

                // Publish the message
                match socket.publish(message).await {
                    Ok(()) => {
                        // Message published successfully
                    }
                    Err((error, _returned_message)) => {
                        println!("PUBLISHER: Failed to publish message: {}", error);
                        // In production, could retry with returned_message
                    }
                }

                // Brief pause between messages
                tokio::time::sleep(std::time::Duration::from_millis(800)).await;
            }
        }

        // Pause between rounds
        println!("PUBLISHER: Completed round, starting next...");
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

/// Run the message subscriber
///
/// The subscriber connects to the publisher and receives messages matching
/// its topic subscriptions. It demonstrates how topic filtering works at
/// the subscriber side.
async fn run_subscriber(
    url: &str,
    topics: &[String],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!("SUBSCRIBER: Connecting to {}", url);

    // Create SUB socket and connect to publisher
    let c_url = CString::new(url)?;
    let socket = pubsub0::Sub0::dial(&c_url).await?;
    let mut ctx = socket.context();

    // Set up topic subscriptions
    if topics.len() == 1 && topics[0].is_empty() {
        println!("SUBSCRIBER: Subscribing to ALL messages (no filtering)");
        ctx.disable_filtering();
    } else {
        println!("SUBSCRIBER: Subscribing to topics: {:?}", topics);
        for topic in topics {
            ctx.subscribe_to(topic.as_bytes());
        }
    }

    // Configure to prefer newer messages if backlog exists
    ctx.prefer_new(true);

    println!("SUBSCRIBER: Waiting for messages... (Press Ctrl+C to stop)");

    let mut message_count = 0;

    // Receive messages in a loop
    loop {
        match ctx.next().await {
            Ok(message) => {
                message_count += 1;

                // Parse and display the received message
                match std::str::from_utf8(message.as_slice()) {
                    Ok(text) => {
                        println!("SUBSCRIBER: [{}] Received: {}", message_count, text);
                    }
                    Err(_) => {
                        println!(
                            "SUBSCRIBER: [{}] Received non-UTF8 message ({} bytes)",
                            message_count,
                            message.len()
                        );
                    }
                }
            }
            Err(error) => {
                println!("SUBSCRIBER: Error receiving message: {}", error);
                // In a real application, might want to reconnect or handle specific errors
                break;
            }
        }
    }

    Ok(())
}
