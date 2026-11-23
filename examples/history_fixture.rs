//! Reproducible synthetic BC1 stream for Python history-policy fixtures.
//! Every block predicts the previous one; block zero references preset 255.
//! Uses bcpack-rs by Ahmet Dedeler's test-only synthetic encoder API.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use bcpack::encode::{Fragment, encode_lanes};
    let lanes: Vec<_> = (0..12)
        .map(|i| {
            vec![Fragment {
                symbols: vec![if i == 0 { 2 } else { 0 }; 512],
                q: if i == 0 { 0 } else { 15 },
                z: 0,
            }]
        })
        .collect();
    let bytes = encode_lanes(&bcpack::bc1::BC1_ALPHABET_BITS, &lanes)?;
    println!("{}", serde_json::to_string(&bytes)?);
    Ok(())
}
