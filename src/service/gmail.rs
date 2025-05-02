use async_imap::{
    error::Error::ConnectionLost,
    extensions::idle::IdleResponse::{ManualInterrupt, NewData, Timeout},
};
use futures_util::StreamExt;
use imap_proto::{
    Address,
    MailboxDatum::{Exists, Flags},
    Response::MailboxData,
};
use tokio::net::TcpStream;
use tokio_native_tls::TlsStream;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, instrument, warn};

use crate::{http, printer::PrintData};

#[derive(Debug, Default)]
struct GmailOAuth2<'a> {
    user: &'a str,
    access_token: &'a str,
}
impl async_imap::Authenticator for GmailOAuth2<'_> {
    type Response = String;

    fn process(&mut self, _challenge: &[u8]) -> Self::Response {
        format!(
            "user={}\x01auth=Bearer {}\x01\x01",
            self.user, self.access_token
        )
    }
}

async fn init_imap_tls_client() -> async_imap::Client<TlsStream<TcpStream>> {
    let stream = TcpStream::connect("imap.gmail.com:993").await.unwrap();

    let tls_stream = tokio_native_tls::TlsConnector::from(native_tls::TlsConnector::new().unwrap())
        .connect("imap.gmail.com", stream)
        .await
        .expect("Unable to initiate TLS connection to server");
    info!("Initiated TLS connection to the server");

    async_imap::Client::new(tls_stream)
}

async fn generate_access_token(
    client_id: &str,
    client_secret: &str,
    refresh_token: &str,
) -> String {
    let client = http::client();
    let params = [
        ("client_id", client_id),
        ("client_secret", client_secret),
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
    ];
    let request = client
        .post("https://oauth2.googleapis.com/token")
        .form(&params)
        .send()
        .await
        .expect("Something went wrong sending refreshing token");

    let response = request
        .json::<serde_json::Value>()
        .await
        .expect("Malformed JSON");

    response["access_token"].as_str().unwrap().to_string()
}

#[instrument(skip(cancel_token, sender))]
pub async fn start_service(
    cancel_token: CancellationToken,
    sender: tokio::sync::mpsc::Sender<PrintData>,
) {
    let user = std::env::var("GMAIL_USER").expect("Gmail user not provided!");
    let refresh_token =
        std::env::var("GMAIL_REFRESH_TOKEN").expect("Gmail refresh token not provided!");
    let client_id = std::env::var("GMAIL_CLIENT_ID").expect("Google App Client ID not provided!");
    let client_secret =
        std::env::var("GMAIL_CLIENT_SECRET").expect("Google App Client Secret not provided!");

    loop {
        if cancel_token.is_cancelled() {
            debug!("Cancel signal caught! Stopping service...");
            break;
        }

        let access_token = generate_access_token(&client_id, &client_secret, &refresh_token).await;

        let authenticator = GmailOAuth2 {
            user: &user,
            access_token: &access_token,
        };

        let mut client = init_imap_tls_client().await;
        // https://github.com/chatmail/async-imap/issues/84
        let _ = client.read_response().await.unwrap();

        let mut session = client
            .authenticate("XOAUTH2", authenticator)
            .await
            .expect("Unable to login to IMAP server!");

        // Select inbox for initial count

        let mailbox = match session.select("INBOX").await {
            Ok(m) => m,
            Err(ConnectionLost) => {
                panic!("IMAP connection lost!")
            }
            Err(_) => {
                panic!("Unknown error!")
            }
        };

        info!(
            "Exists: {} | Recent: {} | Unseen: {:?}",
            mailbox.exists, mailbox.recent, mailbox.unseen
        );

        'idle: loop {
            let mail_num: Option<u32>;
            let mut idle = session.idle();
            idle.init().await.unwrap();

            'wait_new_mail: loop {
                info!("Waiting for mailbox update...");
                let (res, _stop_token) = idle.wait();
                match res.await {
                    Ok(NewData(res)) => {
                        let parsed_res = res.parsed();
                        match parsed_res {
                            MailboxData(Flags(f)) => debug!("IMAP Flag update: {f:?}"),
                            MailboxData(Exists(digit)) => {
                                // Break out of session and read email
                                info!("New email fetched: {digit}");
                                session = idle.done().await.unwrap();
                                mail_num = Some(*digit);
                                break 'wait_new_mail;
                            }

                            u => warn!("Unknown mailbox data: {u:?}"),
                        }
                    }

                    Ok(Timeout) => {
                        debug!("Timed out - No mailbox update in 25 mins. Waiting again...")
                    }

                    // SAFETY: ManualInterrupt should never be used since we're not dropping stop_token
                    Ok(ManualInterrupt) => unreachable!(),

                    Err(ConnectionLost) => {
                        error!("IMAP Connection is lost!");
                        break 'idle;
                    }
                    Err(e) => {
                        panic!("Unknown error while idling: {e}")
                    }
                };
            }

            // Fetch latest email
            let Some(mail_num) = mail_num else {
                error!("No supplied mail number is given. Is there a logic fallthrough case?");
                continue;
            };

            let mut email_stream = session
                .fetch(format!("{mail_num}"), "(RFC822 ENVELOPE)")
                .await
                .expect("Unknown error while fetching latest email");
            while let Some(Ok(mail)) = email_stream.next().await {
                let envelope = mail.envelope().expect("Email does not contain envelope!");

                let subject = envelope
                    .subject
                    .as_ref()
                    .map_or("None".to_string(), |subject| {
                        String::from_utf8_lossy(subject).to_string()
                    });
                let body = mail
                    .body()
                    .map_or("<No email body provided>".to_string(), |body| {
                        String::from_utf8_lossy(body).to_string()
                    });
                let from = generate_email_list(envelope.from.as_ref().unwrap());
                let to = generate_email_list(envelope.to.as_ref().unwrap());

                info!("Subject: {subject}\n{body}",);
                sender
                    .send(PrintData {
                        title: subject,
                        subtitle: Some(format!("From: {from}\nTo: {to}")),
                        message: Some(body),
                        timestamp: Default::default(),
                    })
                    .await
                    .unwrap()
            }
            drop(email_stream); // Just in case
        }
    }
}

fn generate_email_list(addresses: &Vec<Address<'_>>) -> String {
    addresses
        .iter()
        .map(|a| {
            a.name.as_ref().map_or(
                String::from_utf8_lossy(a.mailbox.as_ref().unwrap()).to_string(),
                |n| String::from_utf8_lossy(n.as_ref()).to_string(),
            )
        })
        .collect::<Vec<String>>()
        .join(", ")
}
