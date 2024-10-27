use tokio_util::{sync::CancellationToken, task::TaskTracker};
use tracing::info;

mod http;
mod service;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt::init();

    let task_tracker = TaskTracker::new();
    let cancel_token = CancellationToken::new();

    info!("Starting Notifi-printer...");

    {
        let cancel = cancel_token.clone();
        task_tracker.spawn(service::github::start_service(cancel));
    }

    tokio::signal::ctrl_c()
        .await
        .expect("Unable to listen to CTRL + C signal!");
    info!("CTRL + C signal caught! Stopping all tasks...");
    cancel_token.cancel();
    task_tracker.close();

    task_tracker.wait().await;
    info!("All tasks closed. Goodbye o/")
}
