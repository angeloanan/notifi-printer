use criterion::{criterion_group, criterion_main, Criterion};

pub const ESC: u8 = 0x1B;
pub const GS: u8 = 0x1D;
pub const LF: u8 = 0x0A;

pub const JUSTIFY_LEFT: &[u8; 3] = &[ESC, b'a', 0x0];
pub const JUSTIFY_CENTER: &[u8; 3] = &[ESC, b'a', 0x1];
pub const JUSTIFY_RIGHT: &[u8; 3] = &[ESC, b'a', 0x2];

fn human_extend_from_slice() -> Vec<u8> {
    let mut out: Vec<u8> = vec![ESC, b'@']; // Initialize print
    out.extend_from_slice(&[ESC, b'M', 0x01]); // Uses smaller character font

    // Extend_from_slice might just slowing things down too much
    out.extend_from_slice(JUSTIFY_CENTER); // Set center
    out.extend_from_slice(&[GS, b'!', 0x11]); // Set character size to 2x2
    out.extend_from_slice("New Github Issue".as_bytes()); // Send title
    out.extend_from_slice(&[LF]); // Print

    out.extend_from_slice(&[GS, b'!', 0x00]); // Set character size to 1x1
    out.extend_from_slice(JUSTIFY_LEFT); // Set justify left

    out.extend_from_slice(&[ESC, b'd', 0x00]); // Feed 1 lines

    out.extend_from_slice("Repo: angeloanan/notifi-printer".as_bytes()); // Send subtitle
    out.extend_from_slice(&[LF]); // Print

    out.extend_from_slice([b'-'].repeat(64).as_slice()); // Send line
    out.extend_from_slice(&[LF]); // Print

    out
}

fn minimize_extend_from_slice() -> Vec<u8> {
    // Initialize print
    // Uses smaller character font,
    let mut out: Vec<u8> = vec![ESC, b'@', ESC, b'M', 0x01];
    out.extend_from_slice(JUSTIFY_LEFT);
    out.extend_from_slice(&[GS, b'!', 0x11]);
    out.extend_from_slice("New Github Issue".as_bytes()); // Send title

    out.extend_from_slice(&[LF, GS, b'!', 0x00]);
    out.extend_from_slice(JUSTIFY_LEFT);
    out.extend_from_slice(&[ESC, b'd', 0x00]); // Feed 1 lines
    out.extend_from_slice("Repo: angeloanan/notifi-printer".as_bytes()); // Send subtitle
    out.extend_from_slice(&[LF]); // Print

    out.extend_from_slice([b'-'].repeat(64).as_slice()); // Send line
    out.extend_from_slice(&[LF]); // Print

    out
}

fn vec_push() {
    let mut out: Vec<u8> = Vec::new();
    out.push(ESC);
    out.push(b'@');

    // Smaller character font
    out.push(ESC);
    out.push(b'M');
    out.push(0x01);

    // Justify center
    out.push(ESC);
    out.push(b'a');
    out.push(0x01);

    // Charsize 2x2
    out.push(GS);
    out.push(b'!');
    out.push(0x11);

    // Title
    "New Github Issue"
        .as_bytes()
        .iter()
        .for_each(|c| out.push(*c));
    out.push(LF); // Print

    // Charsize 1x1
    out.push(GS);
    out.push(b'!');
    out.push(0x00);

    // Justify left
    out.push(ESC);
    out.push(b'a');
    out.push(0x00);

    // Feed 1 line
    out.push(ESC);
    out.push(b'd');
    out.push(0x00);

    // Subtitle
    "Repo: angeloanan/notifi-printer"
        .as_bytes()
        .iter()
        .for_each(|c| out.push(*c));
    out.push(LF);

    out.resize(out.len() + 64, b'-');
    // for _ in 0..64 {
    //     out.push(b'-');
    // }
    out.push(LF);
}

fn bench_fibs(c: &mut Criterion) {
    let mut group = c.benchmark_group("Print data merging");

    group.bench_function("Extend from slice (Human Verbose)", |b| {
        b.iter(human_extend_from_slice)
    });

    group.bench_function("Extend from slice (Minimization)", |b| {
        b.iter(minimize_extend_from_slice)
    });

    group.bench_function("Vec::push", |b| {
        b.iter(vec_push);
    });

    group.finish();
}

criterion_group!(benches, bench_fibs);
criterion_main!(benches);
