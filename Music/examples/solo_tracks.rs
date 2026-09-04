//! Renders each track of the default score on its own.
//!
//! A mix can only be diagnosed part by part: an out-of-key pitch class in the
//! sum says nothing about which instrument put it there.

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory: PathBuf = std::env::args()
        .nth(1)
        .unwrap_or_else(|| ".".to_owned())
        .into();
    std::fs::create_dir_all(&directory)?;

    let score = teledra_music::default_score();
    for (index, track) in score.tracks.iter().enumerate() {
        let mut solo = score.clone();
        solo.tracks = vec![track.clone()];
        let audio = solo.render()?;
        let slug: String = track
            .name
            .to_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        let path = directory.join(format!("solo_{index}_{slug}.wav"));
        teledra_music::write_wav(&path, &audio)?;
        println!("{}", path.display());
    }
    Ok(())
}
