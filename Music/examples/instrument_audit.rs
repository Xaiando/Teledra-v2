//! Export the same complete instrument catalog the composer and Studio receive.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::fmt::Write as _;
    use teledra_music::Instrument;

    let mut groups = std::collections::BTreeMap::new();
    for &instrument in Instrument::ALL {
        groups
            .entry(instrument.group())
            .or_insert_with(Vec::new)
            .push(instrument);
    }
    let mut report = format!(
        "# Teledra instrument catalog\n\n{} synthesized instruments. These are the exact voices \
         available to the Organist and the Studio. Ranges are the renderer's working MIDI registers. \
         Harmony voices play native chord voicings; other pitched voices play authored melodic notes. \
         Timpani is tuned; the Percussion role uses unpitched hits.\n\n",
        Instrument::ALL.len()
    );
    for (group, instruments) in groups {
        writeln!(
            report,
            "## {group}\n\n| Instrument | Score ID | Role | MIDI range | Attack | Use |\n|---|---|---|---|---|---|"
        )?;
        for instrument in instruments {
            let info = instrument.info();
            let (attack, _) = instrument.patch_summary();
            writeln!(
                report,
                "| {} | `{}` | {:?} | {}–{} | {attack} | {} |",
                info.name, info.id, info.role, info.range[0], info.range[1], info.description
            )?;
        }
        report.push('\n');
    }
    if let Some(path) = std::env::args().nth(1).map(std::path::PathBuf::from) {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, report)?;
        let json = path.with_extension("json");
        let metadata = Instrument::ALL
            .iter()
            .map(|voice| voice.info())
            .collect::<Vec<_>>();
        std::fs::write(&json, serde_json::to_string_pretty(&metadata)?)?;
        println!("{}\n{}", path.display(), json.display());
    } else {
        print!("{report}");
    }
    Ok(())
}
