use std::net::TcpStream;

use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use serde_json::json;
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, instrument, warn};

use crate::printer::PrintData;

const EVENT_SUBSCRIPTION_URL: &str = "https://api.twitch.tv/helix/eventsub/subscriptions";
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
                    match message {
                        Message::Text(data) => {
                            let data = serde_json::from_str::<serde_json::Value>(&data)
                                .expect("Twitch stream did not return valid JSON");
                            let serde_json::Value::String(message_type) = &data["metadata"]["message_type"] else {
                                error!("Twitch message is missing message_type\n{data}\nSkipping...");
                                continue;
                            };
                            match message_type.as_str() {
                                "session_keepalive" => {
                                    debug!("Keepalive message got");
                                }
                                "notification" => {
                                    debug!("Got a notification message!")
                                }
                                _ => {}
                            };
                        }
                        Message::Close(_) => {
                            // TODO: Twitch ended connection
                            debug!("Twitch ended websocket connection");
                            break;
                        },
                        _ => (),
                    }
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
