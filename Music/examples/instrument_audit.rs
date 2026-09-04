//! Renders one sustained note per instrument and reports what it sounds like.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    use teledra_music::Instrument::*;
    let all = [
        SubBass, Bass, Cello, Organ, Piano, Harp, Pad, Keys, Reed, Pluck, Lead, Flute, Bell, Arp,
    ];
    println!(
        "{:<9} {:>10} {:>9} {:>9}",
        "voice", "register", "attack", "decays"
    );
    for instrument in all {
        let (low, high) = instrument.natural_range();
        let patch = instrument.patch_summary();
        println!(
            "{:<9} {:>4}-{:<5} {:>9} {:>9}",
            format!("{instrument:?}"),
            low,
            high,
            patch.0,
            patch.1
        );
    }
    Ok(())
}
