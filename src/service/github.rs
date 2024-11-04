use std::{str::FromStr, time::Duration};

use chrono::DateTime;
use reqwest::{
    header::{ACCEPT, IF_MODIFIED_SINCE, LAST_MODIFIED},
    StatusCode,
};
use serde_json::json;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, instrument, trace};

use crate::{http, printer::PrintData};

const HTTP_ENDPOINT: &str = "https://api.github.com/notifications";

#[instrument(skip(cancel_token, sender))]
pub async fn start_service(
    cancel_token: CancellationToken,
    sender: tokio::sync::mpsc::Sender<PrintData>,
) {
    let http_client = http::client();
    let mut last_modified_time: Option<Box<str>> = None;

    loop {
        if cancel_token.is_cancelled() {
            info!("Stopping service due to cancel token...");
            break;
        }

        trace!("Building new request");
        let mut req = http_client
            .get(HTTP_ENDPOINT)
            .bearer_auth(std::env::var("GITHUB_PAT").expect("GITHUB_PAT env var is not set!"))
            .header(ACCEPT, "application/vnd.github.v3+json")
            .header("X-GitHub-Api-Version", "2022-11-28");

        // Add Last modified time for long polling; Recommended by GitHub's API docs
        // https://docs.github.com/en/rest/activity/notifications?apiVersion=2022-11-28#about-github-notifications
        if let Some(last_modified_time) = &last_modified_time {
            debug!("Using last modified time: {last_modified_time}");
            req = req.header(IF_MODIFIED_SINCE, last_modified_time.to_string());
        }

        trace!("Sending HTTP request");
        let res = req.send().await;
        if let Err(e) = res {
            error!("Error on sending HTTP request\n{e}");
            tokio::time::sleep(Duration::from_secs(5)).await;
            continue;
        }

        let res = res.unwrap();
        let poll_interval = res.headers().get("X-Poll-Interval").map_or(60, |h| {
            h.to_str().unwrap().to_string().parse::<u64>().unwrap()
        });
        if let Some(header) = res.headers().get(LAST_MODIFIED) {
            let time = header.to_str().unwrap().to_string();
            debug!("Next request using Last-Modified header: {time:?}");
            last_modified_time = Some(time.into_boxed_str());
        };

        let res = res.json::<serde_json::Value>().await.unwrap();
        // info!("{}", res);

        for notif in res.as_array().expect("GitHub returned malformed JSON data") {
            info!(
                "New notification with ID: {}",
                notif["id"].as_str().unwrap()
            );

            match notif["reason"].as_str().unwrap() {
                "manual" | "comment" | "author" => {
                    let latest_comment_url =
                        notif["subject"]["latest_comment_url"].as_str().unwrap();
                    let latest_comment_req = http_client
                        .get(latest_comment_url)
                        .bearer_auth(
                            std::env::var("GITHUB_PAT").expect("GITHUB_PAT env var is not set!"),
                        )
                        .header(ACCEPT, "application/vnd.github.v3+json")
                        .header("X-GitHub-Api-Version", "2022-11-28")
                        .send()
                        .await
                        .expect("Unable to fetch latest comment data");
                    let latest_comment_data: serde_json::Value =
                        latest_comment_req.json().await.unwrap();
                    let updated_time = notif["updated_at"].as_str().unwrap();
                    let thread_id = notif["id"].as_str();
                    //

                    sender
                        .send(PrintData {
                            title: "GitHub: New Issue Comment".to_string(),
                            subtitle: Some(format!(
                                "Repo: {}\n{}",
                                notif["repository"]["full_name"].as_str().unwrap(),
                                notif["subject"]["title"].as_str().unwrap(),
                            )),
                            message: Some(format!(
                                "{}:\n{}",
                                latest_comment_data["user"]["login"].as_str().unwrap(),
                                latest_comment_data["body"].as_str().unwrap(),
                            )),
                            timestamp: DateTime::from_str(updated_time).unwrap(),
                        })
                        .await
                        .unwrap();

                    // Mark notif as read
                    if let Some(thread_id) = thread_id {
                        let req = http_client
                            .patch(format!(
                                "https://api.github.com/notifications/threads/{thread_id}"
                            ))
                            .bearer_auth(
                                std::env::var("GITHUB_PAT")
                                    .expect("GITHUB_PAT env var is not set!"),
                            )
                            .header(ACCEPT, "application/vnd.github.v3+json")
                            .header("X-GitHub-Api-Version", "2022-11-28")
                            .send()
                            .await
                            .unwrap();

                        assert!(req.status() == StatusCode::RESET_CONTENT);
                    }
                }

                other => {
                    error!("Unhandled notification reason: {other}");
                }
            }
        }

        tokio::select! {
            _ = cancel_token.cancelled() => {
                debug!("Cancel signal caught! Stopping service...");
                break;
            }
            _ = tokio::time::sleep(Duration::from_secs(poll_interval)) => {}
        }
    }
}
