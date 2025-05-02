use tokio::{io::AsyncWriteExt, net::TcpStream};
use tracing::{debug, info};

pub const ESC: u8 = 0x1B;
pub const GS: u8 = 0x1D;
pub const LF: u8 = 0x0A;
pub const INDENT: u8 = 0xDD;

pub const JUSTIFY_LEFT: &[u8; 3] = &[ESC, b'a', 0x0];
pub const JUSTIFY_CENTER: &[u8; 3] = &[ESC, b'a', 0x1];
pub const JUSTIFY_RIGHT: &[u8; 3] = &[ESC, b'a', 0x2];

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt::init();

    let addr = std::env::var("PRINTER_ADDR").expect("Env `PRINTER_ADDR` not set!");
    let mut printer_stream = TcpStream::connect(&addr)
        .await
        .expect("Unable to connect to {addr}");
    info!("Opened a TCP Stream @ {addr}");

    let mut out: Vec<u8> = vec![ESC, b'@']; // Initialize print
                                            // out.extend_from_slice(&[GS, b'b', 0x01]); // Enable font smoothing

    // Charset 0x00
    // out.extend_from_slice(&[ESC, b'M', 0x00]); // Uses smaller character font
    out.extend_from_slice("Character Code Set 0x00\u{0A}".as_bytes());
    out.extend_from_slice("0 0123456789ABCDEF0123456789ABCDEF\u{0A}".as_bytes());
    for i in 0..8 {
        out.extend_from_slice(format!("{} ", i * 2).as_bytes());
        for j in 0..=32 {
            out.push((i * 32) + j);
        }
        out.push(LF);
    }

    out.extend_from_slice(&[ESC, b'd', 0x04, LF]); // Feed 5

    out.extend_from_slice(&[ESC, b'M', 0x01]); // Uses smaller character font
    out.extend_from_slice("Character Code Set 0x01\u{0A}".as_bytes());
    out.extend_from_slice("0 0123456789ABCDEF0123456789ABCDEF\u{0A}".as_bytes());
    for i in 0..8 {
        out.extend_from_slice(format!("{} ", i * 2).as_bytes());
        for j in 0..32 {
            out.push((i * 32) + j);
        }
        out.push(LF);
    }

    out.extend_from_slice(&[ESC, b'd', 0x06, LF]);
    out.extend_from_slice(&[ESC, b'i']);
    out.extend_from_slice(&[0x0C]);

    printer_stream.write_all(&out).await.unwrap();
}
