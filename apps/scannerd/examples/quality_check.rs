//! Judges photographs as a capture would: `cargo run --example quality_check -- *.jpg`.
fn main() {
    for path in std::env::args().skip(1) {
        let started = std::time::Instant::now();
        match std::fs::read(&path)
            .map_err(anyhow::Error::from)
            .and_then(|jpeg| scannerd::quality::assess_jpeg(&jpeg))
        {
            Ok(q) => println!(
                "{path}: {} (ink {}, paper {}, sharpness {:?}) in {} ms",
                q.summary(),
                q.ink,
                q.paper,
                q.sharpness,
                started.elapsed().as_millis()
            ),
            Err(err) => println!("{path}: {err:#}"),
        }
    }
}
