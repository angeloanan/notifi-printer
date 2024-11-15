use std::time::Duration;

use imap::extensions::idle::WaitOutcome;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, instrument};

use crate::printer::PrintData;

#[instrument(skip(cancel_token, sender))]
pub async fn start_service(
    cancel_token: CancellationToken,
    sender: tokio::sync::mpsc::Sender<PrintData>,
) {
    let domain = std::env::var("IMAP_DOMAIN").unwrap();
    let port = std::env::var("IMAP_PORT")
        .unwrap()
        .parse::<u16>()
        .expect("Invalid IMAP_PORT! Port is not an u16!");
    let username = std::env::var("IMAP_USER").unwrap();
    let password = std::env::var("IMAP_PASSWORD").unwrap();

    let client = imap::connect(
        (domain.clone(), port),
        &domain,
        &native_tls::TlsConnector::new().unwrap(),
    )
    .unwrap();

    let mut session = client
        .login(username, password)
        .expect("Unable to login to IMAP server!");

    // session.list(None, None).unwrap().iter().for_each(|m| {
    //     info!("Mailbox {m:?} exists");
    // });

    session
        .select("INBOX")
        .expect("Unable to select main mailbox!");

    loop {
        if cancel_token.is_cancelled() {
            break;
        }

        let result = tokio::task::block_in_place(|| {
            session
                .idle()
                .expect("IMAP server does not support the IDLE command!")
                .wait_with_timeout(Duration::from_secs(10))
                .unwrap()
        });

        if result == WaitOutcome::TimedOut {
            debug!("No new email...");
            continue;
        }

        // Handle new email
        todo!();
    }
}
