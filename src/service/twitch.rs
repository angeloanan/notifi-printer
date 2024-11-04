use std::str::FromStr;
use tracing::instrument;

use chrono::DateTime;
use futures_util::StreamExt;
use serde_json::{json, Value::String};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, trace};

use crate::printer::PrintData;

const EVENT_SUBSCRIPTION_URL: &str = "https://api.twitch.tv/helix/eventsub/subscriptions";
const CHANNEL_INFO_URL: &str = "https://api.twitch.tv/helix/channels?broadcaster_id=";

const BROADCASTER_IDS: [&str; 1] = [
    "88547576", // RTGame
];

#[instrument(skip(cancel_token, sender))]
pub async fn start_service(
    cancel_token: CancellationToken,
    sender: tokio::sync::mpsc::Sender<PrintData>,
) {
    // Connect URL may change dynamically via a Reconnect Message
    // https://dev.twitch.tv/docs/eventsub/handling-websocket-events#reconnect-message
    let mut connect_url = Box::new("wss://eventsub.wss.twitch.tv/ws?keepalive_timeout_seconds=600");

    let reqwest = crate::http::client();

    loop {
        if cancel_token.is_cancelled() {
            break;
        }

        let client_request = connect_url.into_client_request().unwrap();
        let (mut stream, _response) = tokio_tungstenite::connect_async_tls_with_config(
            client_request,
            None,
            true,
            Some(tokio_tungstenite::Connector::NativeTls(
                native_tls::TlsConnector::new().unwrap(),
            )),
        )
        .await
        .unwrap();

        // Skip 1, first message is Ping - Calling .skip() consumes the stream for some reason.
        // Need to discover & refactor on how to do this properly
        stream.next().await;
        let Some(Ok(message)) = stream.next().await else {
            // TODO: Handle this properly
            panic!("Websocket instantly closed")
        };

        // TODO: Handle this properly
        let welcome_text = message.into_text().unwrap();
        let welcome_message = serde_json::from_str::<serde_json::Value>(&welcome_text)
            .expect("Welcome message contains malformed JSON");

        // Extract session id and subscribe to event
        let session_id = &welcome_message["payload"]["session"]["id"];
        let subscription_body = json!({
            "type": "stream.online",
            "version": "1",
            "condition": { "broadcaster_user_id": BROADCASTER_IDS[0] },
            "transport": { "method": "websocket", "session_id": session_id }
        });

        let subscription_request = reqwest
            .post(EVENT_SUBSCRIPTION_URL)
            // https://twitchapps.com/tmi/
            .header("Client-Id", "q6batx0epp608isickayubi39itsckt")
            .bearer_auth(
                std::env::var("TWITCH_OAUTH_TOKEN").expect("Env var TWITCH_OAUTH_TOKEN is missing; Generate one on https://twitchapps.com/tmi/"),
            )
            .json(&subscription_body)
            .send()
            .await
            .expect("Unable to subscribe to Twitch Event");
        debug!("Subscription status: {}", subscription_request.status());

        tokio::pin!(stream);

        loop {
            tokio::select! {
                _ = cancel_token.cancelled() => {
                    break;
                }
                Some(Ok(message)) = stream.next() => {
                    if message.is_close() {
                        // TODO: Twitch ended connection
                        debug!("Twitch ended websocket connection");
                        break;
                    }

                    let data = message
                        .into_text()
                        .expect("Twitch sent a non-string-able data");
                    let data = serde_json::from_str::<serde_json::Value>(&data)
                        .expect("Twitch stream did not return valid JSON");
                    let String(message_type) = &data["metadata"]["message_type"] else {
                        error!("Twitch message is missing message_type\n{data}\nSkipping...");
                        continue;
                    };
                    match message_type.as_str() {
                        "session_keepalive" => {
                            trace!("Keepalive message got");
                        }
                        "notification" => {
                            info!("Got a notification message!");

                            // Directly assume that event will be `stream.online`
                            // Handle more events here when I do add more ws events
                            let String(channel_id) = &data["payload"]["event"]["broadcaster_user_id"]
                            else {
                                error!("Twitch notification is missing `broadcaster_user_id`\n{data}\nSkipping...");
                                continue;
                            };

                            // Get channel info for stream title, category & game details
                            let channel_info_req = reqwest
                                .get(format!("{CHANNEL_INFO_URL}{channel_id}"))
                                .send()
                                .await
                                .expect("Unable to fetch more streamer detail");
                            let Ok(channel_info) = channel_info_req.json::<serde_json::Value>().await
                            else {
                                error!("Unable to parse Twitch Channel Info JSON");
                                continue;
                            };
                            let channel_info = channel_info["data"].as_array().unwrap().first().unwrap();

                            sender
                                .send(PrintData {
                                    title: format!(
                                        "Twitch: {} is Live",
                                        channel_info["broadcaster_name"].as_str().unwrap()
                                    ),
                                    subtitle: Some(channel_info["title"].as_str().unwrap().to_string()),
                                    message: Some(format!(
                                        "Category: {}\nTags: {:?}",
                                        channel_info["game_name"].as_str().unwrap(),
                                        channel_info["tags"].as_array().unwrap()
                                    )),
                                    timestamp: DateTime::from_str(
                                        data["metadata"]["message_timestamp"].as_str().unwrap(),
                                    )
                                    .unwrap(),
                                })
                                .await
                                .unwrap();
                        }

                        other => {
                            error!("Unhandled message type: {other}");
                        }
                    };
                }
            }
        }

        // Check if we break out of loop because of cancel token
        if cancel_token.is_cancelled() {
            debug!("Cancel signal caught! Stopping service...");
            break;
        }
    }
}
