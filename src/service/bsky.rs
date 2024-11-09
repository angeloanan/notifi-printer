use std::{str::FromStr, time::Duration};

use reqwest::StatusCode;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, instrument};

use crate::{http, printer::PrintData};

#[instrument(skip(cancel_token, sender))]
pub async fn start_service(
    cancel_token: CancellationToken,
    sender: tokio::sync::mpsc::Sender<PrintData>,
) {
    let reqwest = http::client();

    // None = Expired
    let mut access_token: Option<Box<str>> = None;
    // None = New
    let mut refresh_jwt: Option<Box<str>> = None;

    loop {
        if cancel_token.is_cancelled() {
            debug!("Cancel signal caught! Stopping service...");
            return;
        }

        // Refresh JWT is None if initial run
        if refresh_jwt.is_none() {
            let session = create_session(reqwest.clone()).await;
            access_token = Some(session.0);
            refresh_jwt = Some(session.1);
        }

        // Refresh Access Token if expired
        if access_token.is_none() {
            let session = refresh_session(reqwest.clone(), &refresh_jwt.unwrap()).await;
            access_token = Some(session.0);
            refresh_jwt = Some(session.1);
        }

        let unread_notifications =
            get_unread_notifications(reqwest.clone(), access_token.as_ref().unwrap()).await;

        // Token expired - Set access token to none & continue loop
        if matches!(unread_notifications, Err(BskyError::ExpiredToken)) {
            access_token = None;
            continue;
        }
        let unread_notifications = unread_notifications.unwrap();
        debug!("Unread notif count: {}", unread_notifications.len());

        for n in unread_notifications {
            info!("New unread notif: {n}");
            let notif_type = n["reason"]
                .as_str()
                .expect("Notification does not have field `reason`");
            let timestamp = n["record"]["createdAt"].as_str().unwrap();
            let print_data: PrintData = match notif_type {
                "follow" => {
                    let display_name = n["author"]["displayName"].as_str().unwrap();
                    let handle = n["author"]["handle"].as_str().unwrap();

                    PrintData {
                        title: "Bsky: New follower".to_string(),
                        subtitle: None,
                        message: Some(format!("{display_name} ({handle}) followed you",)),
                        timestamp: chrono::DateTime::from_str(timestamp).unwrap(),
                    }
                }

                "reply" => {
                    let display_name = n["author"]["displayName"].as_str().unwrap();
                    let handle = n["author"]["handle"].as_str().unwrap();
                    let text = n["record"]["text"].as_str().unwrap();

                    PrintData {
                        title: "Bsky: New reply".to_string(),
                        subtitle: None,
                        message: Some(format!("{display_name} ({handle}) said:\n{text}")),
                        timestamp: chrono::DateTime::from_str(timestamp).unwrap(),
                    }
                }

                // Noop
                "like" => {
                    let display_name = n["author"]["displayName"].as_str().unwrap();
                    let handle = n["author"]["handle"].as_str().unwrap();

                    PrintData {
                        title: "Bsky: New like".to_string(),
                        subtitle: None,
                        message: Some(format!("{display_name} ({handle}) liked your post")),
                        timestamp: chrono::DateTime::from_str(timestamp).unwrap(),
                    }
                }

                _ => {
                    error!("Unknown notification reason caught: {notif_type}");
                    continue;
                }
            };

            sender.send(print_data).await.unwrap();
            reqwest
                .post("https://bsky.social/xrpc/app.bsky.notification.updateSeen")
                .json(&json!({ "seenAt": timestamp  }))
                .send()
                .await
                .unwrap();
        }

        tokio::select! {
            _ = cancel_token.cancelled() => {}
            _ = tokio::time::sleep(Duration::from_secs(10)) => {}
        }
    }
}

const CREATE_SESSION_URL: &str =
    "https://lionsmane.us-east.host.bsky.network/xrpc/com.atproto.server.createSession";
/// # Panic
///
/// * Panics on HTTP request fails
/// * Panics on malformed data returned from backend
async fn create_session(client: reqwest::Client) -> (Box<str>, Box<str>) {
    let id = std::env::var("BSKY_IDENTIFIER").expect("Envvar BSKY_IDENTIFIER not supplied!");
    let pass = std::env::var("BSKY_PASSWORD").expect("Envvar BSKY_PASSWORD not supplied!");

    let req = client
        .post(CREATE_SESSION_URL)
        .json(&json!({
            "identifier": id,
            "password": pass
        }))
        .send()
        .await
        .unwrap();

    assert!(req.status() == StatusCode::OK);
    let res: Value = req.json().await.unwrap();
    let access_jwt = res["accessJwt"]
        .as_str()
        .expect("Session's `accessJwt` is missing!")
        .to_string()
        .into_boxed_str();
    let refresh_jwt = res["refreshJwt"]
        .as_str()
        .expect("Session's `refreshJwt` is missing!")
        .to_string()
        .into_boxed_str();

    (access_jwt, refresh_jwt)
}

const REFRESH_SESSION_URL: &str =
    "https://lionsmane.us-east.host.bsky.network/xrpc/com.atproto.server.refreshSession";
/// # Panic
///
/// * Panics on HTTP request fails
/// * Panics on malformed data returned from backend
async fn refresh_session(client: reqwest::Client, refresh_token: &str) -> (Box<str>, Box<str>) {
    debug!("Refreshing session token");

    let req = client
        .post(REFRESH_SESSION_URL)
        .header("Authorization", refresh_token)
        .send()
        .await
        .unwrap();

    debug!("request status: {}", req.status());
    assert!(req.status() == StatusCode::OK);

    let res: Value = req.json().await.unwrap();
    let access_jwt = res["accessJwt"]
        .as_str()
        .expect("Session's `accessJwt` is missing!")
        .to_string()
        .into_boxed_str();
    let refresh_jwt = res["refreshJwt"]
        .as_str()
        .expect("Session's `refreshJwt` is missing!")
        .to_string()
        .into_boxed_str();

    (access_jwt, refresh_jwt)
}

#[derive(Debug)]
enum BskyError {
    ExpiredToken,
}

const LIST_NOTIFICATION_URL: &str =
    "https://bsky.social/xrpc/app.bsky.notification.listNotifications";
async fn get_unread_notifications(
    client: reqwest::Client,
    access_token: &str,
) -> Result<Vec<Value>, BskyError> {
    let req = client
        .get(LIST_NOTIFICATION_URL)
        .bearer_auth(access_token)
        .send()
        .await
        .unwrap();

    // If token is expired / invalid, status code is BadRequest
    if req.status() == StatusCode::BAD_REQUEST {
        return Err(BskyError::ExpiredToken);
    }

    let text = req.text().await.unwrap();
    // println!("{text}");

    let res: Value = serde_json::from_str(&text).unwrap();
    // assert!(req.status() == StatusCode::OK);

    let notifications = res["notifications"].as_array().unwrap();
    Ok(notifications
        .iter()
        .filter(|n| !n["isRead"].as_bool().unwrap_or(true))
        .map(std::borrow::ToOwned::to_owned)
        .collect())
}
