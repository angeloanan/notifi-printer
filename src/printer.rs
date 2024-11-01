use chrono::{DateTime, Local};
use tokio::{io::AsyncWriteExt, net::TcpStream, sync::mpsc::Receiver};
use tokio_util::sync::CancellationToken;
use tracing::{debug, instrument};

pub struct PrintData {
    pub title: String,
    pub subtitle: Option<String>,

    pub message: Option<String>,
    pub timestamp: DateTime<Local>,
}

const ESC: u8 = 0x1B;
const GS: u8 = 0x1D;
const LF: u8 = 0x0A;

#[instrument(skip(cancel, printer, receiver))]
pub async fn process_prints(
    cancel: CancellationToken,
    mut printer: TcpStream,
    mut receiver: Receiver<PrintData>,
) {
    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                debug!("Cancel signal caught! Stopping service...");
                break;
            }

            Some(data) = receiver.recv() => {
                printer.write_all(&[ESC, b'@']).await.unwrap(); // Initialize Printer

                printer.write_all(&[ESC, b'a', 0x1]).await.unwrap(); // Set centering
                printer.write_all(&[GS, b'!', 0x22]).await.unwrap(); // Set character size to 3x3
                printer.write_all(data.title.as_bytes()).await.unwrap(); // Send title
                printer.write_all(&[LF]).await.unwrap(); // Print title
                printer.write_all(&[ESC, b'a', 0x0]).await.unwrap(); // Set justify left

                if let Some(subtitle) = data.subtitle {
                    printer.write_all(&[GS, b'!', 0x11]).await.unwrap(); // Set character size to 2x2
                    printer.write_all(subtitle.as_bytes()).await.unwrap(); // Send subtitle
                    printer.write_all(&[LF]).await.unwrap(); // Print title
                }

                printer.write_all(&[ESC, b'a', 0x0]).await.unwrap(); // Set justify left
                printer.write_all(&[GS, b'!', 0x00]).await.unwrap(); // Set character size to normal (1x1)

                // Print timestamp
                let human_time = data.timestamp.format("%B %e, %r");
                let timestamp_line = format!("Created at: {human_time}");
                printer.write_all(timestamp_line.as_bytes()).await.unwrap(); // Send timestamp_line
                printer.write_all(&[LF]).await.unwrap(); // Print timestamp

                if let Some(message) = data.message {
                    printer.write_all(&[ESC, b'd', 0x01]).await.unwrap(); // Feed 2 lines

                    let processed_message = message
                        .trim()
                        .chars()
                        .map(|c| {
                            if c.is_whitespace() && c != ' ' {
                                return LF;
                            }
                            c as u8
                        })
                        .collect::<Vec<u8>>();
                    printer
                        .write_all(processed_message.as_slice())
                        .await
                        .unwrap();
                    printer.write_all(&[LF]).await.unwrap(); // Print final line if haven't
                }

                // Closing
                printer.write_all(&[ESC, b'd', 0x04]).await.unwrap(); // Feed 4 lines
                printer.write_all(&[ESC, b'i']).await.unwrap(); // Full cut
                printer.write_all(&[ESC, b'd', 0x04]).await.unwrap(); // Feed 4 lines
                printer.write_all(&[0x0C]).await.unwrap(); //Print and return to standard mode in page mode
            }
        }
    }
}
