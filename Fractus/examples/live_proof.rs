//! Renders and decodes a live Fractus GIF to prove visible frame changes.

use std::io::BufReader;
use std::path::PathBuf;

use image::AnimationDecoder;
use teledra_fractus::Scene;

fn digest(bytes: impl IntoIterator<Item = u8>) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("fractus-live-proof.gif"));
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let scene = Scene::parse(
        "version 2\ntype=moire\nwidth=600\nheight=800\niterations=180\nseed=1371413118\npalette=electric_cyan\nlayers=moire@0.62@screen@electric_cyan;rose_curve@0.28@add@solar_gold",
    )?;
    let manifest = scene.render_gif(&output, 24)?;
    let decoder =
        image::codecs::gif::GifDecoder::new(BufReader::new(std::fs::File::open(&output)?))?;
    let frames = decoder.into_frames().collect_frames()?;
    let digests = frames
        .iter()
        .map(|frame| digest(frame.buffer().as_raw().iter().copied()))
        .collect::<Vec<_>>();
    let distinct = digests
        .iter()
        .collect::<std::collections::HashSet<_>>()
        .len();
    println!(
        "{}",
        serde_json::json!({
            "output": output,
            "frames": frames.len(),
            "distinct_frames": distinct,
            "first_frame": digests.first(),
            "last_frame": digests.last(),
            "non_black_pixels": manifest.non_black_pixels,
            "verified": manifest.verified,
        })
    );
    if frames.len() != 24 || distinct < 12 || !manifest.verified {
        return Err("live Fractus proof did not contain enough distinct verified frames".into());
    }
    Ok(())
}
