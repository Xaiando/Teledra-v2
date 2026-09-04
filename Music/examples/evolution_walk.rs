//! Walks the native evolution forward and reports how often it moves the anchors.
//!
//! A variation lands every eighteen to thirty-six seconds, so this is what a listener
//! actually experiences over a long stream.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let steps: usize = args
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(40);

    // Pass a score path to walk the piece the court is actually playing. The
    // default score is the ideal case; a live lineage is the real one.
    let mut score = match args.next() {
        Some(path) => {
            teledra_music::Score::from_json(&std::fs::read_to_string(path)?)?.mastered()?
        }
        None => teledra_music::default_score(),
    };

    let (mut key_moves, mut tempo_moves, mut voice_moves) = (0, 0, 0);
    let voicing = |score: &teledra_music::Score| {
        score
            .tracks
            .iter()
            .map(|track| track.instrument().label())
            .collect::<Vec<_>>()
            .join(" ")
    };
    let mut heard: std::collections::BTreeSet<&'static str> = score
        .tracks
        .iter()
        .map(|t| t.instrument().label())
        .collect();

    println!("   n    bpm  tonic         mode       movement craft  voicing");
    for step in 0..steps {
        let next = score.evolved()?;
        if next.tonic != score.tonic || format!("{:?}", next.mode) != format!("{:?}", score.mode) {
            key_moves += 1;
        }
        if next.bpm != score.bpm {
            tempo_moves += 1;
        }
        if voicing(&next) != voicing(&score) {
            voice_moves += 1;
        }
        score = next;
        heard.extend(score.tracks.iter().map(|t| t.instrument().label()));
        if step < 20 {
            let craft = score.composition_report();
            println!(
                "{:>4}  {:>5} {:>6} {:>11} {:>14} {:>3}/100  {}",
                step + 1,
                score.bpm,
                score.tonic,
                format!("{:?}", score.mode),
                craft.movement.label(),
                craft.quality,
                voicing(&score)
            );
        }
    }
    println!(
        "\nover {steps} variations (~{:.0} min of stream): key/mode moved {key_moves} times, tempo {tempo_moves}, voicing {voice_moves}",
        steps as f32 * 30.0 / 60.0
    );
    println!(
        "voices heard: {}",
        heard.into_iter().collect::<Vec<_>>().join(", ")
    );
    Ok(())
}
