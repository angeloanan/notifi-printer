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

    let mut out: Vec<u8> = vec![]; // Initialize print
    // out.extend_from_slice(&[GS, b'V', 49]);
    // let mut out: Vec<u8> = vec![ESC, b'@']; // Initialize print
    // out.extend_from_slice(&[GS, b'b', 0x01]); // Enable font smoothing

    out.extend_from_slice("Vention CEY (1M.5)\n".as_bytes());
    out.extend_from_slice("April 9, 2024\n".as_bytes());

    // Full cut and introduces a empty line.
    // Must be cut 2 lines in for seamless top / bottom
    // The cutter and the printer head in my thermal printer is somehow offset. So:
    // ---- Cut
    // 
    //
    // ---- Print head
    // This unfortunately means that there will always be wasted space / paper in between prints :(
    // Interestingly, we could optimize this process for mass-printing.
    // If you need to not introduce a line here, omit the previous `\n` or replace it with `LF`
    out.extend_from_slice(&[ESC, b'i']);

    out.extend_from_slice("DC 5.5*2.5 or DC 5.5*2.1\n".as_bytes());
    out.extend_from_slice("5V 3A (15W)\n".as_bytes());

    // Cut, feed 4 
    out.extend_from_slice(&[ESC, b'd', 0x04]);
    out.extend_from_slice(&[ESC, b'i']);
    out.extend_from_slice(&[0x0C]);

    printer_stream.write_all(&out).await.unwrap();
}
