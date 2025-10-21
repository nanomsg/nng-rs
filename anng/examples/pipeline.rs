//! # Pipeline Work Distribution
//!
//! Demonstrates the Push/Pull messaging pattern for distributing work across multiple workers.
//! This pattern excels at load balancing tasks without requiring acknowledgment.
//!
//! The distributor (Push socket) sends tasks to workers (Pull sockets), with NNG automatically
//! load balancing across available workers. Each task goes to exactly one worker.
//!
//! ## Usage
//!
//! ```bash
//! # Terminal 1: Start distributor
//! cargo run --example pipeline distributor tcp://127.0.0.1:7000
//!
//! # Terminal 2-4: Start workers (run multiple for load balancing)
//! cargo run --example pipeline worker tcp://127.0.0.1:7000 1
//! cargo run --example pipeline worker tcp://127.0.0.1:7000 2
//! cargo run --example pipeline worker tcp://127.0.0.1:7000 3
//! ```
//!
//! ## Key Protocol Features
//!
//! - **Load balancing**: Tasks distributed round-robin across workers
//! - **No acknowledgment**: Fire-and-forget semantics for maximum throughput
//! - **Natural backpressure**: Slow workers receive fewer tasks automatically
//! - **Horizontal scaling**: Add workers dynamically without configuration changes

use anng::{protocols::pipeline0, Message};
use std::ffi::CString;
use std::io::Write;
use std::time::Duration;

#[derive(Debug)]
enum Mode {
    Distributor { task_count: u32 },
    Worker { worker_id: u32 },
}

#[derive(Debug)]
struct Args {
    mode: Mode,
    url: String,
}

fn parse_args() -> Result<Args, Box<dyn std::error::Error + Send + Sync>> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        eprintln!("Usage: {} <distributor|worker> [worker_id] [URL]", args[0]);
        eprintln!();
        eprintln!("Demonstrates Pipeline (Push/Pull) work distribution pattern.");
        eprintln!();
        eprintln!("Arguments:");
        eprintln!("  distributor         Run as task distributor (sends 20 tasks)");
        eprintln!("  worker [ID]         Run as worker with optional ID (default: 1)");
        eprintln!("  [URL]              URL to connect to (default: tcp://127.0.0.1:7000)");
        eprintln!();
        eprintln!("Examples:");
        eprintln!("  {} distributor", args[0]);
        eprintln!("  {} worker 1", args[0]);
        eprintln!("  {} worker 2 tcp://127.0.0.1:7000", args[0]);
        std::process::exit(1);
    }

    let mode = match args[1].as_str() {
        "distributor" | "dist" => Mode::Distributor { task_count: 20 },
        "worker" => {
            let worker_id = if args.len() > 2 && !args[2].starts_with("tcp://") {
                args[2].parse().unwrap_or(1)
            } else {
                1
            };
            Mode::Worker { worker_id }
        }
        other => {
            return Err(format!("expected 'distributor' or 'worker', got '{}'", other).into());
        }
    };

    let url = args
        .iter()
        .skip(2)
        .find(|s| s.starts_with("tcp://") || s.starts_with("ipc://"))
        .cloned()
        .unwrap_or_else(|| "tcp://127.0.0.1:7000".to_string());

    Ok(Args { mode, url })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Initialize tracing for better debugging
    tracing_subscriber::fmt::init();

    let args = parse_args()?;

    match args.mode {
        Mode::Distributor { task_count } => run_distributor(&args.url, task_count).await,
        Mode::Worker { worker_id } => run_worker(&args.url, worker_id).await,
    }
}

/// Run the task distributor
///
/// The distributor creates and distributes image processing tasks to available workers.
/// It demonstrates realistic work distribution patterns including task variety,
/// prioritization, and timing control.
async fn run_distributor(
    url: &str,
    task_count: u32,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!("DISTRIBUTOR: Starting on {}", url);
    println!("DISTRIBUTOR: Will distribute {} tasks", task_count);

    // Create PUSH socket and listen for worker connections
    let c_url = CString::new(url)?;
    let mut socket = pipeline0::Push0::listen(&c_url).await?;

    // Configure send buffer for better performance with burst traffic
    socket.set_send_buffer(100)?;

    println!("DISTRIBUTOR: Waiting for workers to connect...");
    tokio::time::sleep(Duration::from_secs(3)).await;

    let mut tasks_sent = 0u32;

    for task_id in 1..=task_count {
        // Create simple math task
        let number = task_id * 7 + 13; // Generate some variety in numbers
        let task_text = format!("square {}", number);
        let mut message = Message::with_capacity(task_text.len());
        message.write_all(task_text.as_bytes())?;

        println!("DISTRIBUTOR: Sending task {}: {}", task_id, task_text);

        // Send task to available worker
        match socket.push(message).await {
            Ok(()) => {
                tasks_sent += 1;
            }
            Err((error, _message)) => {
                println!("DISTRIBUTOR: Failed to send task {}: {}", task_id, error);
                // In production: implement retry logic with exponential backoff
            }
        }

        // Brief pause between tasks for better demo visibility
        tokio::time::sleep(Duration::from_millis(300)).await;
    }

    println!("DISTRIBUTOR: Finished distributing {} tasks", tasks_sent);

    // Keep distributor alive briefly to ensure final messages are delivered
    tokio::time::sleep(Duration::from_secs(2)).await;

    Ok(())
}

/// Run a worker process
///
/// Workers receive tasks from the distributor and simulate processing them.
/// This demonstrates realistic worker patterns including variable processing
/// times, error handling, and graceful shutdown.
async fn run_worker(
    url: &str,
    worker_id: u32,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!("WORKER {}: Connecting to distributor at {}", worker_id, url);

    // Create PULL socket and connect to distributor
    let c_url = CString::new(url)?;
    let mut socket = pipeline0::Pull0::dial(&c_url).await?;

    println!("WORKER {}: Ready to process tasks", worker_id);

    let mut tasks_processed = 0u32;
    let mut total_processing_time = Duration::ZERO;

    // Process tasks in a loop until distributor disconnects
    loop {
        match socket.pull().await {
            Ok(message) => {
                let task_start = std::time::Instant::now();

                // Parse the received task
                let task_text = std::str::from_utf8(message.as_slice())?;
                if let Some(task_parts) = task_text.strip_prefix("square ") {
                    match task_parts.parse::<u32>() {
                        Ok(number) => {
                            tasks_processed += 1;
                            println!("WORKER {}: Processing task - square {}", worker_id, number);

                            // Simulate processing work
                            let processing_time =
                                Duration::from_millis(200 + (worker_id % 3) as u64 * 100);
                            tokio::time::sleep(processing_time).await;

                            let result = number * number;
                            let actual_duration = task_start.elapsed();
                            total_processing_time += actual_duration;

                            println!(
                                "WORKER {}: Completed - {} squared = {} (took {:?})",
                                worker_id, number, result, actual_duration
                            );
                        }
                        Err(e) => {
                            println!("WORKER {}: Failed to parse number: {}", worker_id, e);
                        }
                    }
                } else {
                    println!(
                        "WORKER {}: Unknown task format: {} (received {} bytes)",
                        worker_id,
                        task_text,
                        message.len()
                    );
                }
            }
            Err(error) => {
                println!("WORKER {}: Error receiving task: {}", worker_id, error);
                // In production: implement exponential backoff and reconnection logic
                break;
            }
        }
    }

    let avg_processing_time = if tasks_processed > 0 {
        total_processing_time / tasks_processed
    } else {
        Duration::ZERO
    };

    println!(
        "WORKER {}: Processed {} tasks, average time: {:?}",
        worker_id, tasks_processed, avg_processing_time
    );

    Ok(())
}
