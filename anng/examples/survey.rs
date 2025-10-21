//! # Survey-Based Service Discovery
//!
//! Demonstrates the Surveyor/Respondent messaging pattern for distributed service discovery,
//! health monitoring, and consensus building. This pattern excels when you need to gather
//! information from multiple nodes with optional responses.
//!
//! The surveyor broadcasts questions to all connected respondents, who may optionally reply
//! within a timeout window. Responses arrive asynchronously and can be processed as they come.
//!
//! ## Usage
//!
//! ```bash
//! # Terminal 1: Start service discovery coordinator
//! cargo run --example survey surveyor tcp://127.0.0.1:8000
//!
//! # Terminal 2-5: Start various services (run multiple for realistic scenario)
//! cargo run --example survey respondent tcp://127.0.0.1:8000 --service-type web-server --node-id web-1
//! cargo run --example survey respondent tcp://127.0.0.1:8000 --service-type database --node-id db-1
//! cargo run --example survey respondent tcp://127.0.0.1:8000 --service-type cache --node-id cache-1
//! cargo run --example survey respondent tcp://127.0.0.1:8000 --service-type worker --node-id worker-1
//! ```
//!
//! ## Key Protocol Features
//!
//! - **Broadcast queries**: One surveyor reaches all connected respondents
//! - **Optional responses**: Respondents choose whether to reply to each survey
//! - **Time-bounded**: Surveys have configurable timeouts for timely results
//! - **Asynchronous collection**: Process responses as they arrive

use anng::{Message, protocols::survey0};
use std::ffi::CString;
use std::io::Write;
use std::time::Duration;

#[derive(Debug)]
enum Mode {
    Surveyor,
    Respondent {
        service_type: String,
        node_id: String,
    },
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
            "Usage: {} <surveyor|respondent> [service_type] [node_id] [URL]",
            args[0]
        );
        eprintln!();
        eprintln!("Demonstrates Survey/Respondent pattern for service discovery.");
        eprintln!();
        eprintln!("Arguments:");
        eprintln!("  surveyor            Run as service discovery coordinator");
        eprintln!("  respondent          Run as service instance");
        eprintln!("  [service_type]      Type of service (default: service)");
        eprintln!("  [node_id]           Unique node identifier (default: node-1)");
        eprintln!("  [URL]              URL to connect to (default: tcp://127.0.0.1:8000)");
        eprintln!();
        eprintln!("Examples:");
        eprintln!("  {} surveyor", args[0]);
        eprintln!("  {} respondent web-server api-1", args[0]);
        eprintln!(
            "  {} respondent database db-1 tcp://127.0.0.1:8000",
            args[0]
        );
        std::process::exit(1);
    }

    let mode = match args[1].as_str() {
        "surveyor" | "coordinator" => Mode::Surveyor,
        "respondent" | "service" => {
            let service_type = args
                .get(2)
                .filter(|s| !s.starts_with("tcp://") && !s.starts_with("ipc://"))
                .cloned()
                .unwrap_or_else(|| "service".to_string());

            let node_id = args
                .get(3)
                .filter(|s| !s.starts_with("tcp://") && !s.starts_with("ipc://"))
                .cloned()
                .unwrap_or_else(|| "node-1".to_string());

            Mode::Respondent {
                service_type,
                node_id,
            }
        }
        other => {
            return Err(format!("expected 'surveyor' or 'respondent', got '{}'", other).into());
        }
    };

    let url = args
        .iter()
        .skip(2)
        .find(|s| s.starts_with("tcp://") || s.starts_with("ipc://"))
        .cloned()
        .unwrap_or_else(|| "tcp://127.0.0.1:8000".to_string());

    Ok(Args { mode, url })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Initialize tracing for better debugging
    tracing_subscriber::fmt::init();

    let args = parse_args()?;

    match args.mode {
        Mode::Surveyor => run_surveyor(&args.url).await,
        Mode::Respondent {
            service_type,
            node_id,
        } => run_respondent(&args.url, &service_type, &node_id).await,
    }
}

/// Run the service discovery coordinator
///
/// The surveyor performs various types of service discovery and health monitoring
/// by broadcasting surveys and collecting responses. It demonstrates practical
/// patterns for distributed system coordination.
async fn run_surveyor(url: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!(
        "SURVEYOR: Starting service discovery coordinator on {}",
        url
    );

    // Create SURVEYOR socket and listen for service connections
    let c_url = CString::new(url)?;
    let socket = survey0::Surveyor0::listen(&c_url).await?;
    let mut ctx = socket.context();

    println!("SURVEYOR: Waiting for services to connect...");
    tokio::time::sleep(Duration::from_secs(5)).await;

    // Perform various types of service discovery surveys
    let survey_types = ["ping", "status", "info"];

    for survey_type in &survey_types {
        println!("\n=== {} survey ===", survey_type);

        // Create simple text-based survey
        let mut message = Message::with_capacity(survey_type.len());
        message.write_all(survey_type.as_bytes())?;

        println!("SURVEYOR: Broadcasting {} survey", survey_type);

        // Configure timeout
        let timeout = Duration::from_secs(3);

        // Send survey and collect responses
        match ctx.survey(message, timeout).await {
            Ok(mut responses) => {
                let mut response_count = 0;
                let start_time = std::time::Instant::now();

                // Collect responses until timeout
                while let Some(response_result) = responses.next().await {
                    match response_result {
                        Ok(response_msg) => {
                            response_count += 1;
                            let response_text = std::str::from_utf8(response_msg.as_slice())?;
                            let response_time = start_time.elapsed();

                            println!(
                                "SURVEYOR: [{}] {} ({:?})",
                                response_count, response_text, response_time
                            );
                        }
                        Err(error) => {
                            println!("SURVEYOR: Survey error: {}", error);
                            break;
                        }
                    }
                }

                let total_time = start_time.elapsed();
                println!(
                    "SURVEYOR: Collected {} responses in {:?}",
                    response_count, total_time
                );
            }
            Err((error, _message)) => {
                println!("SURVEYOR: Failed to send survey: {}", error);
            }
        }

        // Wait between different survey types
        tokio::time::sleep(Duration::from_secs(3)).await;
    }

    println!("\nSURVEYOR: Completed all surveys");
    Ok(())
}

/// Run a service respondent
///
/// The respondent represents a service instance that participates in service discovery
/// and responds to health checks and other coordination activities. It demonstrates
/// realistic service behavior patterns.
async fn run_respondent(
    url: &str,
    service_type: &str,
    node_id: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!(
        "RESPONDENT: Starting {} service '{}' connecting to {}",
        service_type, node_id, url
    );

    // Create RESPONDENT socket and connect to surveyor
    let c_url = CString::new(url)?;
    let socket = survey0::Respondent0::dial(&c_url).await?;
    let mut ctx = socket.context();

    println!("RESPONDENT: Connected and ready to respond to surveys");

    // Listen for surveys and respond appropriately
    loop {
        match ctx.next_survey().await {
            Ok((survey_msg, responder)) => {
                let survey_text = std::str::from_utf8(survey_msg.as_slice())?;
                println!("RESPONDENT: Received {} survey", survey_text);

                // Decide whether to respond based on query type
                let should_respond = match survey_text {
                    "ping" => true,
                    "status" => true,
                    "info" => service_type != "cache", // Cache services don't respond to info
                    _ => false,
                };

                if should_respond {
                    // Create simple text response
                    let response_text = format!("{}-{}", service_type, node_id);
                    let mut response_msg = Message::with_capacity(response_text.len());
                    response_msg.write_all(response_text.as_bytes())?;

                    match responder.respond(response_msg).await {
                        Ok(()) => {
                            println!("RESPONDENT: Sent {} response", survey_text);
                        }
                        Err((_, error, _)) => {
                            println!("RESPONDENT: Failed to send response: {}", error);
                        }
                    }
                } else {
                    println!(
                        "RESPONDENT: Ignoring {} survey (not applicable)",
                        survey_text
                    );
                    // Drop the responder to not respond
                }
            }
            Err(error) => {
                println!("RESPONDENT: Error receiving survey: {}", error);
                // In production: implement reconnection logic
                break;
            }
        }
    }

    Ok(())
}
