//! Compact shared context for executable PyMusic generation.

pub(crate) fn song_generation_context(
    report: &serde_json::Value,
    feedback: &str,
    learning: &str,
) -> String {
    let evidence = serde_json::json!({"title":report["title"],"metadata":report["metadata"],
        "duration_seconds":report["duration_seconds"],"layers":report["composition_audit"]["layer_stats"],
        "notes_scheduled":report["composition_audit"]["notes_scheduled"],"notes_mixed":report["composition_audit"]["notes_mixed"]});
    format!(
        "You compose complete music through our local PyMusic song recipe. Choose real themes, answers, rests, harmonic motion and contrasting sections for a 30–107 second piece. The supplied schema describes exact musical controls and available voices. The 72-instrument catalog and seven classic PyMusic voices remain available, with pitched voices for melodic/harmonic roles and percussion available as texture. The court plays its accepted music while this candidate develops in the background; complete verified results enter its library. Listener Keep/Reject is separate from technical validity. Follow the latest musical brief.\nPLAYING PIECE FACTS (not the new recipe): {evidence}\nLISTENER VERDICTS: {feedback}\n{learning}"
    )
}

pub(crate) fn generation_context(
    report: &serde_json::Value,
    feedback: &str,
    learning: &str,
) -> String {
    let catalog = teledra_music::Instrument::ALL
        .iter()
        .map(|voice| {
            let info = voice.info();
            format!("{} (MIDI {}-{})", info.id, info.range[0], info.range[1])
        })
        .collect::<Vec<_>>()
        .join(", ");
    let facts = serde_json::json!({
        "title": report["title"],
        "key": report["metadata"]["key"],
        "bpm": report["metadata"]["bpm"],
        "bars": report["metadata"]["bars"],
        "duration_seconds": report["duration_seconds"],
        "notes_requested": report["notes_requested"],
        "notes_rendered": report["notes_rendered"],
    });
    format!(
        "You are the PyMusic composer. Produce executable musical changes from the current brief. Keep independent layers, recognizable phrases, rests, contrasting sections and harmonic arrivals. Preserve working note placement and the accepted key, tempo and sample rate during a revision. A changed title, comment or metadata alone is not musical development.\nPYTHON API: import numpy as np; from teledra_synth import *. synth_note('C4', 0.5, wave_type='triangle', volume=0.15, sr=SR) retains classic PyMusic synthesis. instrument_note('C4', 0.5, instrument='piano', volume=0.15, sr=SR) accesses the 72 voices below. Numeric pitches are Hz, never MIDI. The second argument is total duration in seconds, including release. V1 helpers default to 44100 Hz; preserve the existing SR or pass it consistently. mix_waves(track, note, start_time=seconds, sr=SR) returns the updated additive mix; retain it. Notes at different start times create phrases; adding arrays at zero creates a chord. If existing code uses place(), retain that working placement helper. A newly synthesized note must actually be mixed. Update any matching event declaration when changing its start or duration. Keep 4-600 seconds of complete audio. Use NumPy, math and teledra_synth only. No device/file/network/process APIs. Call play_sound(full_track, sr=SR, loop=True) exactly once after composing all layers. This exports verified audio to the single court player.\nAVAILABLE 72 INSTRUMENTS (comfortable MIDI ranges; pass note names to the Python API): {catalog}\nACCEPTED RENDER FACTS: {facts}\nNAMED OPERATOR VERDICTS:\n{}\n{}",
        feedback, learning,
    )
}
