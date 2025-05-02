use std::fmt::Write as _;
use std::str::FromStr;

use axum::{routing::post, Json};
use chrono::DateTime;
use serde::{Deserialize, Serialize};
use tokio::select;
use tokio_util::sync::CancellationToken;
use tower_http::cors::CorsLayer;
use tracing::{debug, error, instrument};

use crate::printer::PrintData;

#[instrument(skip(cancel_token, sender))]
pub async fn start_service(
    cancel_token: CancellationToken,
    sender: tokio::sync::mpsc::Sender<PrintData>,
) {
    let router = axum::Router::new()
        .route(
            "/discord/notifications",
            post(move |body| handle_notifications(body, sender)),
        )
        .layer(CorsLayer::very_permissive());

    let listener = tokio::net::TcpListener::bind("0.0.0.0:42276")
        .await
        .unwrap();

    // TODO: Make this a global axum listener
    select! {
      () = cancel_token.cancelled() => {
        debug!("Cancel signal caught! Stopping service...");
      }
      Err(e) = axum::serve(listener, router) => {
        error!("Axum crashed - {e}");
      }
    }
}

#[derive(Serialize, Deserialize)]
struct DiscordNotificationDto {
    pub username: String,
    pub global_name: String,

    pub channel_name: Option<String>,
    pub guild_name: Option<String>,

    pub message: String,
    pub timestamp: String,
}

async fn handle_notifications(
    Json(body): Json<DiscordNotificationDto>,
    sender: tokio::sync::mpsc::Sender<PrintData>,
) {
    let mut message = match body.channel_name {
        Some(name) => {
            format!("Channel: {name}")
        }
        None => {
            format!("Channel: {}'s DMs", body.username)
        }
    };

    if let Some(guild_name) = body.guild_name {
        write!(message, "\nGuild: {guild_name}").unwrap();
    }

    write!(
        message,
        "\n\n{} ({}): {}",
        body.global_name, body.username, body.message
    )
    .unwrap();

    sender
        .send(PrintData {
            title: "Discord: New Notification".to_string(),
            subtitle: None,
            message: Some(message),
            timestamp: DateTime::from_str(&body.timestamp).unwrap(),
        })
        .await
        .unwrap();
}
