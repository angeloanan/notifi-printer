use std::{str::FromStr, time::Duration};
use tracing::instrument;

use chrono::DateTime;
use futures_util::StreamExt;
use serde_json::{json, Value::String};
use tokio_tungstenite::tungstenite::{protocol::WebSocketConfig, ClientRequestBuilder, Message};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info};

use crate::printer::PrintData;

const EVENT_SUBSCRIPTION_URL: &str = "https://api.twitch.tv/helix/eventsub/subscriptions";
const CHANNEL_INFO_URL: &str = "https://api.twitch.tv/helix/channels?broadcaster_id=";

const BROADCASTER_IDS: [&str; 4] = [
    "88547576",  // RTGame
    "57220741",  // CakeJumper
    "132141901", // narpy
    "60679655",  // Jitizm12301
];

const DEFAULT_WS_URL: &str = "wss://eventsub.wss.twitch.tv/ws?keepalive_timeout_seconds=30";

#[instrument(skip(cancel_token, sender))]
pub async fn start_service(
    cancel_token: CancellationToken,
    sender: tokio::sync::mpsc::Sender<PrintData>,
) {
    // Connect URL may change dynamically via a Reconnect Message
    // https://dev.twitch.tv/docs/eventsub/handling-websocket-events#reconnect-message
    let mut custom_connect_url: Option<Box<str>> = None;

    let reqwest = crate::http::client();

    loop {
        let client_request = ClientRequestBuilder::new(
            custom_connect_url
                .as_ref()
                .unwrap_or(&DEFAULT_WS_URL.to_string().into_boxed_str())
                .parse()
                .unwrap(),
        );
        let (mut stream, _response) = tokio_tungstenite::connect_async_tls_with_config(
            client_request,
            Some(WebSocketConfig {
                accept_unmasked_frames: true,
                ..Default::default()
            }),
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
        // info!("Welcome message: {welcome_text}");
        let welcome_message = serde_json::from_str::<serde_json::Value>(&welcome_text)
            .expect("Welcome message contains malformed JSON");

        // Extract session id and subscribe to event
        let session_id = &welcome_message["payload"]["session"]["id"];
        info!("Session ID: {session_id}");
        if custom_connect_url.is_none() {
            // Default connect url = needs to (re)register subscriptions
            for id in BROADCASTER_IDS {
                let subscription_body = json!({
                    "type": "stream.online",
                    "version": "1",
                    "condition": { "broadcaster_user_id": id },
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
                debug!(
                    "Subscription status for user {id}: {}",
                    subscription_request.status()
                );
                let sub_res = subscription_request.text().await.unwrap();
                debug!("{sub_res}");
            }
        }

        tokio::pin!(stream);

        loop {
            tokio::select! {
                () = cancel_token.cancelled() => {
                    debug!("Cancel signal caught! Stopping service...");
                    let _ = stream.close(None).await;
                    break;
                }

                // When client doesn't receive an event or keepalive message for longer
                // than keepalive_timeout_seconds, Assume that the connection is lost
                // They said 30s, but due to latency imma be safe and put it at 40s
                () = tokio::time::sleep(Duration::from_secs(40)) => {
                    error!("Didn't get any message for 40s, closing connection & reconnecting...");
                    let _ = stream.close(None).await;
                    // Also assume that session ID is gone
                    custom_connect_url = None;

                    break;
                }

                // Handle message normally
                // Will be out of loop if stream is None or contains Err
                // TODO: Handle if contains Err
                Some(Ok(message)) = stream.next() => {
                    match message {
                        Message::Text(data) => {
                            // info!("{data}");
                            let data = serde_json::from_str::<serde_json::Value>(&data)
                                .expect("Twitch stream did not return valid JSON");
                            let String(message_type) = &data["metadata"]["message_type"] else {
                                error!("Twitch message is missing message_type\n{data}\nSkipping...");
                                continue;
                            };
                            match message_type.as_str() {
                                "session_keepalive" => {
                                    // debug!("Keepalive message got");
                                }

                                "session_reconnect" => {
                                    info!("Twitch sent reconnecting message!");
                                    // TODO: Twitch docs says "You should not close the old connection until you receive a Welcome message on the new connection"
                                    // I cba to implement this with current code structure, so i'm just gonna remake the connection from scratch

                                    // let reconnect_url = data["payload"]["session"]["reconnect_url"].as_str().unwrap();
                                    // custom_connect_url = Some(reconnect_url.to_string().into_boxed_str());
                                    break;
                                }

                                "notification" => {
                                    info!("Got a notification message!");

                                    // Directly assume that event will be `stream.online`
                                    // Handle more events here when I do add more ws events
                                    info!("Notification message: {data}");
                                    let String(channel_id) = &data["payload"]["event"]["broadcaster_user_id"]
                                    else {
                                        error!("Twitch notification is missing `broadcaster_user_id`\n{data}\nSkipping...");
                                        continue;
                                    };

                                    // Get channel info for stream title, category & game details
                                    let channel_info_req = reqwest
                                        .get(format!("{CHANNEL_INFO_URL}{channel_id}"))
                                        .header("Client-Id", "q6batx0epp608isickayubi39itsckt")
                                        .bearer_auth(std::env::var("TWITCH_OAUTH_TOKEN").unwrap())
                                        .send()
                                        .await
                                        .expect("Unable to fetch more streamer detail");
                                    let Ok(channel_info) = channel_info_req.json::<serde_json::Value>().await
                                    else {
                                        error!("Unable to parse Twitch Channel Info JSON");
                                        continue;
                                    };
                                    info!("Channel info: {channel_info}");
                                    let channel_info = channel_info["data"].as_array().unwrap().first().unwrap();

                                    let stream_title = channel_info["title"].as_str().unwrap().to_string();
                                    let game_name =  channel_info["game_name"].as_str().unwrap();

                                    let tags = channel_info["tags"].as_array().unwrap();
                                    let tags_stringified: Vec<&str> = tags.iter().map(|t| { t.as_str().unwrap() }).collect();
                                    let tags_joined = tags_stringified.join(", ");

                                    sender
                                        .send(PrintData {
                                            title: format!(
                                                "Twitch: {} is Live",
                                                channel_info["broadcaster_name"].as_str().unwrap()
                                            ),
                                            subtitle: None,
                                            message: Some(format!("{stream_title}\n\nCategory: {game_name}\nTags: {tags_joined}")),
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

                        },

                        Message::Ping(_) |  Message::Pong(_) | Message::Frame(_) => {},
                        Message::Binary(vec) => {
                            info!("Twitch set binary message: {:?}", vec);
                        },
                        Message::Close(frame) => {
                            error!("Twitch ended websocket connection");
                            if let Some(frame) = frame {
                                error!("Close frame: {frame:?}");
                            }
                            break;
                        },
                    }

                }
            }
        }

        // Check if we break out of loop because of cancel token
        if cancel_token.is_cancelled() {
            break;
        }
    }
}
