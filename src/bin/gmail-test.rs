use async_imap::{extensions::idle::IdleResponse, types::Capability};
use futures_util::StreamExt;
use imap_proto::{MailboxDatum, Response::MailboxData};
use tokio::net::TcpStream;
use tracing::{info, warn};

#[derive(Clone, Copy, Debug, Default)]
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

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt::init();

    let user = std::env::var("GMAIL_USER").expect("Gmail user not provided!");
    let access_token =
        std::env::var("GMAIL_ACCESS_TOKEN").expect("Gmail access token not provided!");

    let authenticator = GmailOAuth2 {
        user: &user,
        access_token: &access_token,
    };

    let stream = TcpStream::connect(("imap.gmail.com", 993)).await.unwrap();
    info!("Connected to imap.gmail.com");

    let tls_stream = tokio_native_tls::TlsConnector::from(native_tls::TlsConnector::new().unwrap())
        .connect("imap.gmail.com", stream)
        .await
        .expect("Unable to initiate TLS connection to server");
    info!("Initiated TLS connection to the server");

    let mut client = async_imap::Client::new(tls_stream);
    info!("Started IMAP client");

    // Take in gmail's initial response
    let _ = client.read_response().await.unwrap().unwrap();
    let mut session = match client.authenticate("XOAUTH2", authenticator).await {
        Ok(session) => {
            info!("Successfully authenticated as {}!", authenticator.user);
            session
        }
        Err((e, _)) => {
            panic!("Error while authenticating email: {e}")
        }
    };

    let capabilities = session
        .capabilities()
        .await
        .expect("Can't request for server capabilities!");

    assert!(
        capabilities.has(&async_imap::types::Capability::Atom("IDLE".into())),
        "Mailbox does not have the `IDLE` capability!"
    );
    info!(
        "Capabilities: {:?}",
        capabilities.iter().collect::<Vec<&Capability>>()
    );

    // Initial mailbox fetch

    let mailbox = match session.select("INBOX").await {
        Ok(m) => m,
        Err(async_imap::error::Error::ConnectionLost) => {
            panic!("IMAP connection lost!")
        }
        Err(_) => {
            panic!("Unknown error!")
        }
    };

    let mut exists_count = mailbox.exists;

    info!(
        "Exists: {} | Recent: {} | Unseen: {:?}",
        mailbox.exists, mailbox.recent, mailbox.unseen
    );

    loop {
        let mut idle = session.idle();
        match idle.init().await {
            Ok(_) => info!("Initialized IDLE!"),
            Err(async_imap::error::Error::ConnectionLost) => {
                panic!("IMAP connection lost!")
            }
            Err(_) => {
                panic!("Unknown error!");
            }
        }

        let (res, _stop_token) = idle.wait();
        info!("Waiting for IDLE to conclude...");
        match res.await {
            Ok(IdleResponse::NewData(response)) => {
                session = idle.done().await.unwrap();

                let parsed_data = response.parsed();
                match parsed_data {
                    MailboxData(MailboxDatum::Flags(_)) => {}
                    MailboxData(MailboxDatum::Exists(digit)) => {
                        let mut mail_stream =
                            match session.fetch(format!("{digit}"), "(RFC822 ENVELOPE)").await {
                                Ok(s) => s,
                                Err(e) => panic!("Unknown error while fetching latest email: {e}"),
                            };

                        while let Some(Ok(message)) = mail_stream.next().await {
                            let envelope = message.envelope().unwrap();
                            let subject = envelope
                                .subject
                                .as_ref()
                                .map_or("None".to_string(), |subject| {
                                    String::from_utf8_lossy(subject).to_string()
                                });
                            let body = message
                                .body()
                                .as_ref()
                                .map_or("<No email body provided>".to_string(), |body| {
                                    String::from_utf8_lossy(body).to_string()
                                });

                            info!("Subject: {subject}\n{body}",);
                        }
                        drop(mail_stream); // Mut borrow apparently
                    }

                    u => {
                        warn!("Unknown mailbox data: {u:?}")
                    }
                }
            }
            Ok(_) => unreachable!(),
            Err(async_imap::error::Error::ConnectionLost) => {
                panic!("IMAP connection lost while idling!");
            }
            Err(e) => {
                panic!("Unknown error while idling: {e}")
            }
        }
    }

    // Done for now :thumbs_up:
    session.logout().await.expect("Unable to logout.")
}
