use std::{str::FromStr, time::Duration};

use chrono::Utc;
use reqwest::{StatusCode, Url};
use serde::{Deserialize, Serialize};
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
            let Ok(session) = refresh_session(reqwest.clone(), &refresh_jwt.unwrap()).await else {
                error!("Unable to refresh session! Going to remake session from scratch...");
                refresh_jwt = None;
                continue;
            };
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

        if !unread_notifications.is_empty() {
            // Loop over all unreads & print
            for n in unread_notifications {
                info!("Notif: {n}");
                let notif_type = n["reason"]
                    .as_str()
                    .expect("Malformed data: notification does not have field `reason`");
                let timestamp = n["record"]["createdAt"].as_str().unwrap();
                let print_data: PrintData = match notif_type {
                    "follow" => {
                        let did = n["author"]["did"].as_str().unwrap();
                        let profile_info =
                            get_profile_info(reqwest.clone(), access_token.as_ref().unwrap(), did)
                                .await
                                .unwrap();

                        PrintData {
                            title: "Bsky: New follower".to_string(),
                            subtitle: None,
                            message: Some(format!(
                                "{} ({}) followed you\n{}\n{} Following | {} Followers",
                                profile_info.display_name,
                                profile_info.handle,
                                profile_info.description,
                                profile_info.follows_count,
                                profile_info.followers_count
                            )),
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

                    // Noop, too spammy
                    "like" => {
                        // let display_name = n["author"]["displayName"].as_str().unwrap();
                        // let handle = n["author"]["handle"].as_str().unwrap();

                        // PrintData {
                        //     title: "Bsky: New like".to_string(),
                        //     subtitle: None,
                        //     message: Some(format!("{display_name} ({handle}) liked your post")),
                        //     timestamp: chrono::DateTime::from_str(timestamp).unwrap(),
                        // }
                        continue;
                    }

                    _ => {
                        error!("Unknown notification reason caught: {notif_type}");
                        continue;
                    }
                };

                sender.send(print_data).await.unwrap();
            }

            // Update last read notification time
            // If error updating, log the error
            // Potential error: Token expired in-between requests
            if let Err(e) =
                update_last_read_notification(reqwest.clone(), access_token.as_ref().unwrap()).await
            {
                error!("Unable to update last read notifications: {e:?}");
            }
        }

        tokio::select! {
            () = cancel_token.cancelled() => {}
            () = tokio::time::sleep(Duration::from_secs(10)) => {}
        }
    }
}

#[derive(Debug)]
enum BskyError {
    ExpiredToken,
    BadRequest,
}

const CREATE_SESSION_URL: &str = "https://bsky.social/xrpc/com.atproto.server.createSession";
/// # Panic
///
/// * Panics on HTTP request fails
/// * Panics on malformed data returned from backend
#[instrument(skip(client))]
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

const REFRESH_SESSION_URL: &str = "https://bsky.social/xrpc/com.atproto.server.refreshSession";
#[instrument(skip(client, refresh_token))]
async fn refresh_session(
    client: reqwest::Client,
    refresh_token: &str,
) -> Result<(Box<str>, Box<str>), BskyError> {
    debug!("Refreshing session token");

    let req = client
        .post(REFRESH_SESSION_URL)
        .bearer_auth(refresh_token)
        .send()
        .await
        .unwrap();

    if req.status() != StatusCode::OK {
        error!("request status: {}", req.status());
        let res = req.text().await.unwrap();
        error!("request data: {res}");

        return Err(BskyError::BadRequest);
    }

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
    info!("Session token refreshed!");

    Ok((access_jwt, refresh_jwt))
}

const LIST_NOTIFICATION_URL: &str =
    "https://bsky.social/xrpc/app.bsky.notification.listNotifications";
#[instrument(skip(client, access_token))]
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
    match req.status() {
        StatusCode::OK => {
            let text = req.text().await.unwrap();
            // println!("{text}");

            let res: Value = serde_json::from_str(&text).unwrap();

            let notifications = res["notifications"].as_array().unwrap();
            Ok(notifications
                .iter()
                .filter(|n| !n["isRead"].as_bool().unwrap_or(true))
                .map(std::borrow::ToOwned::to_owned)
                .collect())
        }

        StatusCode::BAD_REQUEST => Err(BskyError::ExpiredToken),
        _ => Err(BskyError::BadRequest),
    }
}

const UPDATE_LAST_READ_NOTIFICATION_URL: &str =
    "https://bsky.social/xrpc/app.bsky.notification.updateSeen";
#[instrument(skip(client, access_token))]
async fn update_last_read_notification(
    client: reqwest::Client,
    access_token: &str,
) -> Result<(), BskyError> {
    let request = client
        .post(UPDATE_LAST_READ_NOTIFICATION_URL)
        .bearer_auth(access_token)
        .json(&json!({ "seenAt": Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true) }))
        .send()
        .await
        .unwrap();

    assert!(request.status() == StatusCode::OK);

    Ok(())
}

#[derive(Serialize, Deserialize)]
struct BskyProfile {
    did: String,
    handle: String,

    #[serde(rename = "displayName")]
    display_name: String,
    description: String,

    #[serde(rename = "followersCount")]
    followers_count: u32,
    #[serde(rename = "followsCount")]
    follows_count: u32,
    #[serde(rename = "postsCount")]
    posts_count: u32,

    #[serde(rename = "createdAt")]
    created_at: String,
}

const GET_PROFILE_URL: &str = "https://public.api.bsky.app/xrpc/app.bsky.actor.getProfile";
async fn get_profile_info(
    client: reqwest::Client,
    access_token: &str,
    actor: &str,
) -> Result<BskyProfile, BskyError> {
    let url = Url::parse_with_params(GET_PROFILE_URL, &[("actor", actor)]).unwrap();
    let req = client
        .get(url)
        .bearer_auth(access_token)
        .send()
        .await
        .unwrap();

    if req.status() == StatusCode::UNAUTHORIZED {
        return Err(BskyError::ExpiredToken);
    }

    Ok(req.json::<BskyProfile>().await.unwrap())
}

const GET_POST_THREAD_URL: &str = "https://public.api.bsky.app/xrpc/app.bsky.feed.getPostThread";
async fn get_post_details(
    client: reqwest::Client,
    access_token: &str,
    post_uri: &str,
) -> Result<Value, BskyError> {
    let url = Url::parse_with_params(GET_POST_THREAD_URL, &[("uri", post_uri)]).unwrap();
    let req = client
        .get(url)
        .bearer_auth(access_token)
        .send()
        .await
        .unwrap();

    if req.status() == StatusCode::UNAUTHORIZED {
        return Err(BskyError::ExpiredToken);
    }

    Ok(json!({}))
}
