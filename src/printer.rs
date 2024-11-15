use chrono::{DateTime, Local};
use tokio::{io::AsyncWriteExt, net::TcpStream, sync::mpsc::Receiver};
use tokio_util::sync::CancellationToken;
use tracing::{debug, instrument};

pub const ESC: u8 = 0x1B;
pub const GS: u8 = 0x1D;
pub const LF: u8 = 0x0A;

pub const JUSTIFY_LEFT: &[u8; 3] = &[ESC, b'a', 0x0];
pub const JUSTIFY_CENTER: &[u8; 3] = &[ESC, b'a', 0x1];
pub const JUSTIFY_RIGHT: &[u8; 3] = &[ESC, b'a', 0x2];

pub trait Printable {
    fn into_print_data(self) -> Vec<u8>;
}

/// Default printdata
pub struct PrintData {
    pub title: String,
    pub subtitle: Option<String>,

    pub message: Option<String>,
    pub timestamp: DateTime<Local>,
}
impl Printable for PrintData {
    fn into_print_data(self) -> Vec<u8> {
        let mut out: Vec<u8> = vec![ESC, b'@']; // Initialize print
        out.extend_from_slice(&[GS, b'b', 0x01]); // Enable font smoothing
        out.extend_from_slice(&[ESC, b'M', 0x01]); // Uses smaller character font

        // Extend_from_slice might just slowing things down too much
        out.extend_from_slice(JUSTIFY_CENTER); // Set center
        out.extend_from_slice(&[GS, b'!', 0x11]); // Set character size to 2x2
        out.extend_from_slice(self.title.as_bytes()); // Send title
        out.extend_from_slice(&[LF]); // Print

        out.extend_from_slice(&[ESC, b'd', 0x00]); // Feed 1 line
        out.extend_from_slice(&[ESC, b'M', 0x00]); // Uses default character font
        out.extend_from_slice(&[GS, b'!', 0x00]); // Set character size to 1x1
        out.extend_from_slice(JUSTIFY_LEFT); // Set justify left

        if let Some(subtitle) = self.subtitle.as_ref() {
            out.extend_from_slice(&[ESC, b'd', 0x00]); // Feed 1 lines

            out.extend_from_slice(subtitle.as_bytes()); // Send subtitle
            out.extend_from_slice(&[LF]); // Print

            out.extend_from_slice([b'-'].repeat(48).as_slice()); // Send line
            out.extend_from_slice(&[LF]); // Print
        }

        if let Some(message) = self.message.as_ref() {
            out.extend_from_slice(&[ESC, b'd', 0x01]); // Feed 2 lines

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
            out.extend_from_slice(processed_message.as_slice());
            out.extend_from_slice(&[LF]); // Print final line if haven't
        }

        // Print timestamp
        let human_time = self.timestamp.format("%B %e, %r");
        out.extend_from_slice(&[ESC, b'd', 0x01]); // Feed 2 lines
        let timestamp_line = format!("Timestamp: {human_time}");
        out.extend_from_slice(timestamp_line.as_bytes()); // Send timestamp_line
        out.extend_from_slice(&[LF]); // Print timestamp

        out
    }
}

#[instrument(skip(cancel, printer, receiver))]
pub async fn process_prints(
    cancel: CancellationToken,
    mut printer: TcpStream,
    mut receiver: Receiver<PrintData>,
) {
    loop {
        tokio::select! {
            () = cancel.cancelled() => {
                debug!("Cancel signal caught! Stopping service...");
                break;
            }

            Some(data) = receiver.recv() => {
                printer.write_all(&data.into_print_data()).await.unwrap();

                // Closing
                printer.write_all(&[ESC, b'd', 0x06, LF]).await.unwrap(); // Feed 6 lines
                printer.write_all(&[ESC, b'i']).await.unwrap(); // Full cut; I think the auto cutter is 2 lines(?) behind, so the line above effectively feeds 4 line
                printer.write_all(&[0x0C]).await.unwrap(); // Print and return to standard mode in page mode; Finishes the job
            }
        }
    }
}
