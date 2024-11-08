#![deny(clippy::all)]
#![warn(clippy::pedantic)]
#![warn(clippy::nursery)]
#![warn(clippy::cargo)]
#![warn(clippy::perf)]
#![warn(clippy::complexity)]
#![warn(clippy::style)]

use printer::{process_prints, PrintData};
use tokio::{net::TcpStream, sync::mpsc};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use tracing::{debug, info};

mod http;
mod printer;
mod service;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt::init();

    let task_tracker = TaskTracker::new();
    let cancel_token = CancellationToken::new();

    info!("Starting Notifi-printer...");

    let addr = std::env::var("PRINTER_ADDR").expect("Env `PRINTER_ADDR` not set!");
    let printer_stream = TcpStream::connect(&addr)
        .await
        .expect("Unable to connect to {addr}");
    debug!("Opened a TCP Stream @ {addr}");
    let (sender, receiver) = mpsc::channel::<PrintData>(16);

    {
        let cancel = cancel_token.clone();
        task_tracker.spawn(process_prints(cancel, printer_stream, receiver));
    }

    {
        let cancel = cancel_token.clone();
        let sender = sender.clone();
        task_tracker.spawn(service::github::start_service(cancel, sender));
    }
    {
        let cancel = cancel_token.clone();
        let sender = sender.clone();
        task_tracker.spawn(service::twitch::start_service(cancel, sender));
    }
    {
        let cancel = cancel_token.clone();
        let sender = sender.clone();
        task_tracker.spawn(service::bsky::start_service(cancel, sender));
    }

    tokio::signal::ctrl_c()
        .await
        .expect("Unable to listen to CTRL + C signal!");
    info!("CTRL + C signal caught! Stopping all tasks...");
    cancel_token.cancel();
    task_tracker.close();

    task_tracker.wait().await;
    info!("All tasks closed. Goodbye o/");
}
