//! Offline test helper: synthetic BC6H blocks -> exact RGB half-float bits.
//! Pair with scripts/verify_bc6h.py and an independently compiled bcdec oracle.
use std::{error::Error, fs};
fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() != 4 {
        return Err("usage: bc6h_oracle SYNTHETIC_BLOCKS OUTPUT SIGNED(0|1)".into());
    }
    let signed = match args[3].to_str() {
        Some("0") => false,
        Some("1") => true,
        _ => return Err("signed must be 0 or 1".into()),
    };
    if fs::metadata(&args[1])?.len() > 16 * 1_000_000 {
        return Err("synthetic input exceeds 1M-block limit".into());
    }
    let data = fs::read(&args[1])?;
    if data.is_empty() || data.len() % 16 != 0 {
        return Err("input must contain complete blocks".into());
    }
    let mut output = Vec::with_capacity(data.len() / 16 * 96);
    let mut preview_differences = 0;
    for block in data.chunks_exact(16) {
        let block: &[u8; 16] = block.try_into()?;
        let mut old = [0u32; 16];
        let mut new = [0u32; 16];
        texture2ddecoder::decode_bc6_block(block, &mut old, signed);
        rage_texture::bc6h::decode_image(block, 4, 4, &mut new, signed)?;
        preview_differences += usize::from(old != new);
        for pixel in rage_texture::bc6h::decode_block(block, signed) {
            for half in pixel {
                output.extend_from_slice(&half.to_le_bytes());
            }
        }
    }
    fs::write(&args[2], output)?;
    println!(
        "{}",
        serde_json::json!({"blocks":data.len()/16,"preview_blocks_different_from_texture2ddecoder_0_1_2":preview_differences})
    );
    Ok(())
}
