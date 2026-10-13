//! Exact, transactional source edits for an already accepted Python composition.

use serde::Deserialize;

const MAX_BYTES: usize = 128 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PythonEdit {
    base: String,
    intent: String,
    edits: Vec<Replacement>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Replacement {
    #[serde(default)]
    old: Option<String>,
    #[serde(default)]
    line: Option<usize>,
    new: String,
}

/// A numbered-line edit supplies a body relative to the existing indentation.
/// Addressing is safe only with the exact source-version guard checked below.
fn line_replacement(
    source: &str,
    number: usize,
    new: &str,
) -> Result<(usize, usize, String), String> {
    let mut start = 0;
    let line = source
        .split_inclusive('\n')
        .enumerate()
        .find_map(|(index, text)| {
            if index + 1 == number {
                Some(text)
            } else {
                start += text.len();
                None
            }
        })
        .ok_or_else(|| format!("Python source line {number} does not exist"))?;
    let (body, ending) = if let Some(body) = line.strip_suffix("\r\n") {
        (body, "\r\n")
    } else if let Some(body) = line.strip_suffix('\n') {
        (body, "\n")
    } else {
        (line, "")
    };
    let indentation = &body[..body.len() - body.trim_start_matches([' ', '\t']).len()];
    let trimmed = new.trim_end_matches(['\r', '\n']);
    let mut replacement = String::new();
    if !trimmed.is_empty() {
        // Accept an exact existing outer indent too, without doubling it.
        let already_indented = !indentation.is_empty() && trimmed.starts_with(indentation);
        if trimmed.starts_with([' ', '\t']) && !already_indented {
            return Err(format!(
                "Python line {number} replacement must start with its body or its exact existing indentation"
            ));
        }
        let separator = if ending.is_empty() { "\n" } else { ending };
        for (index, part) in trimmed.split('\n').enumerate() {
            let part = part.strip_suffix('\r').unwrap_or(part);
            let relative = if already_indented {
                part.strip_prefix(indentation).unwrap_or(part)
            } else {
                part
            };
            if replacement
                .len()
                .saturating_add(separator.len())
                .saturating_add(indentation.len())
                .saturating_add(relative.len())
                .saturating_add(ending.len())
                > MAX_BYTES
            {
                return Err("Python edited source must fit within 128 KiB".to_owned());
            }
            if index > 0 {
                replacement.push_str(separator);
            }
            if !relative.is_empty() {
                replacement.push_str(indentation);
                replacement.push_str(relative);
            }
        }
        replacement.push_str(ending);
    }
    if replacement == line {
        return Err(format!(
            "Python line {number} changes no source. Its current body is: {}. Choose a real audible change to the working music.",
            body.trim_start_matches([' ', '\t'])
        ));
    }
    Ok((start, start + line.len(), replacement))
}

/// Stable identity of the exact source bytes, including whitespace.
/// This is a content/version guard, not an authentication mechanism.
pub(crate) fn python_source_identity(code: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in code.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("fnv1a64:{hash:016x}")
}

/// Every `old` passage refers to the original baseline, and must occur exactly
/// once. Replacements must not overlap. Validate the whole transaction before
/// producing a new source; no partial result is returned on failure.
pub(crate) fn apply_python_edit(base_code: &str, payload: &str) -> Result<String, String> {
    if base_code.len() > MAX_BYTES || payload.len() > MAX_BYTES {
        return Err("Python source and edit payload must each fit within 128 KiB".to_owned());
    }
    let edit: PythonEdit = serde_json::from_str(payload)
        .map_err(|error| format!("Invalid Python edit JSON: {error}"))?;
    if edit.base != python_source_identity(base_code) {
        return Err(
            "Python edit baseline is stale; use the current accepted source identity".to_owned(),
        );
    }
    if edit.intent.trim().is_empty() {
        return Err("Python edit needs a brief musical intent".to_owned());
    }
    if !(1..=8).contains(&edit.edits.len()) {
        return Err("Python edit must contain 1 to 8 exact replacements".to_owned());
    }

    let mut replacements = Vec::with_capacity(edit.edits.len());
    let mut result_len = base_code.len();
    for (index, replacement) in edit.edits.iter().enumerate() {
        let number = index + 1;
        let (start, end, new) = match (&replacement.old, replacement.line) {
            (None, Some(line)) => line_replacement(base_code, line, &replacement.new)?,
            (Some(old), None) => {
                let Some(first_character) = old.chars().next() else {
                    return Err(format!("Python edit {number} has an empty old passage"));
                };
                if old == &replacement.new {
                    return Err(format!("Python edit {number} changes no source"));
                }
                let Some(start) = base_code.find(old) else {
                    return Err(format!(
                        "Python edit {number} old passage is absent from the baseline"
                    ));
                };
                // Also reject overlapping matches such as 'aa' occurring twice in 'aaa'.
                if base_code[start + first_character.len_utf8()..].contains(old) {
                    return Err(format!(
                        "Python edit {number} old passage matches more than once"
                    ));
                }
                (start, start + old.len(), replacement.new.clone())
            }
            _ => {
                return Err(format!(
                    "Python edit {number} must specify exactly one old passage or source line"
                ));
            }
        };
        result_len = result_len
            .checked_sub(end - start)
            .and_then(|length| length.checked_add(new.len()))
            .ok_or_else(|| "Python edited source is too large".to_owned())?;
        replacements.push((start, end, new));
    }
    replacements.sort_unstable_by_key(|(start, _, _)| *start);
    if replacements.windows(2).any(|pair| pair[0].1 > pair[1].0) {
        return Err("Python edits overlap; combine them into one exact replacement".to_owned());
    }
    if result_len > MAX_BYTES {
        return Err("Python edited source must fit within 128 KiB".to_owned());
    }

    let mut result = String::with_capacity(result_len);
    let mut cursor = 0;
    for (start, end, replacement) in replacements {
        result.push_str(&base_code[cursor..start]);
        result.push_str(&replacement);
        cursor = end;
    }
    result.push_str(&base_code[cursor..]);
    if result == base_code {
        return Err("Python edit changes no source".to_owned());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PIECE: &str = "lead = ['A4', 'C5', 'E5']\nbass = ['A2', 'E2']\nbreath = 0.25\nplay_sound(mix(lead, bass), loop=True)\n";

    fn line_payload(source: &str, edits: serde_json::Value) -> String {
        serde_json::json!({"base": python_source_identity(source), "intent": "Give the answer more phrase space", "edits": edits}).to_string()
    }

    #[test]
    fn numbered_lines_preserve_raw_baseline_indentation_endings_and_other_notes() {
        let source = "if active:\r\n    answer = 0.5\r\n    lead = 'é'\nplay_sound(piece)";
        let edit = line_payload(
            source,
            serde_json::json!([{"line":2,"new":"answer = 0.75"}]),
        );
        assert_eq!(
            apply_python_edit(source, &edit).unwrap(),
            source.replace("answer = 0.5", "answer = 0.75")
        );
        let already_indented = line_payload(
            source,
            serde_json::json!([{"line":2,"new":"    answer = 0.75\n"}]),
        );
        assert_eq!(
            apply_python_edit(source, &edit).unwrap(),
            apply_python_edit(source, &already_indented).unwrap()
        );
        let multi = line_payload(
            source,
            serde_json::json!([{"line":2,"new":"if answer_ready:\n    answer = 0.75"}]),
        );
        assert!(
            apply_python_edit(source, &multi)
                .unwrap()
                .contains("    if answer_ready:\r\n        answer = 0.75\r\n    lead = 'é'\n")
        );
        assert!(
            apply_python_edit(&format!("{source}\n"), &edit)
                .unwrap_err()
                .contains("stale")
        );
    }

    #[test]
    fn line_edits_reject_invalid_addresses_overlaps_ambiguous_targets_and_noops() {
        for edits in [
            serde_json::json!([{"line":0,"new":"x"}]),
            serde_json::json!([{"line":500,"new":"x"}]),
            serde_json::json!([{"line":3,"old":"breath = 0.25","new":"x"}]),
            serde_json::json!([{"line":3,"new":"breath = 0.5"},{"line":3,"new":"breath = 0.75"}]),
            serde_json::json!([{"line":3,"new":"breath = 0.5"},{"old":"breath = 0.25","new":"breath = 0.75"}]),
        ] {
            assert!(apply_python_edit(PIECE, &line_payload(PIECE, edits)).is_err());
        }
        let no_op = line_payload(PIECE, serde_json::json!([{"line":3,"new":"breath = 0.25"}]));
        let error = apply_python_edit(PIECE, &no_op).unwrap_err();
        assert!(error.contains("changes no source") && error.contains("breath = 0.25"));
        // Identical lines are unambiguous when addressed against a version.
        let repeated = "note()\nnote()\n";
        assert_eq!(
            apply_python_edit(
                repeated,
                &line_payload(repeated, serde_json::json!([{"line":2,"new":"answer()"}]))
            )
            .unwrap(),
            "note()\nanswer()\n"
        );
    }

    #[test]
    fn multiline_indentation_cannot_expand_beyond_the_source_limit() {
        let source = format!("{}note()\n", " ".repeat(65_000));
        let payload = line_payload(
            &source,
            serde_json::json!([{"line":1,"new":"answer()\nanswer()\nanswer()"}]),
        );
        assert!(
            apply_python_edit(&source, &payload)
                .unwrap_err()
                .contains("128 KiB")
        );
    }

    fn payload(code: &str, edits: &[(&str, &str)]) -> String {
        serde_json::json!({
            "base": python_source_identity(code),
            "intent": "Let the answering phrase breathe",
            "edits": edits.iter().map(|(old, new)| serde_json::json!({"old": old, "new": new})).collect::<Vec<_>>()
        }).to_string()
    }

    #[test]
    fn focused_edit_preserves_every_unmodified_note_and_playback_call() {
        let result =
            apply_python_edit(PIECE, &payload(PIECE, &[("breath = 0.25", "breath = 0.5")]))
                .unwrap();
        assert_eq!(result, PIECE.replace("breath = 0.25", "breath = 0.5"));
        assert_eq!(python_source_identity(""), "fnv1a64:cbf29ce484222325");
        assert_eq!(python_source_identity("hello"), "fnv1a64:a430d84680aabd0b");
        assert_ne!(
            python_source_identity(PIECE),
            python_source_identity(&format!("{PIECE}\n"))
        );
    }

    #[test]
    fn stale_baseline_and_ambiguous_passages_are_rejected() {
        let edit = payload(PIECE, &[("breath = 0.25", "breath = 0.5")]);
        assert!(
            apply_python_edit(&format!("{PIECE}# newer\n"), &edit)
                .unwrap_err()
                .contains("stale")
        );
        for (code, old) in [("note note", "note"), ("aaa", "aa"), ("ééé", "éé")] {
            assert!(
                apply_python_edit(code, &payload(code, &[(old, "x")]))
                    .unwrap_err()
                    .contains("more than once")
            );
        }
    }

    #[test]
    fn replacements_share_one_baseline_and_fail_transactionally() {
        let original = PIECE.to_owned();
        let bad = payload(
            &original,
            &[("breath = 0.25", "breath = 0.5"), ("missing note", "A4")],
        );
        assert!(apply_python_edit(&original, &bad).is_err());
        assert_eq!(original, PIECE);
        let result = apply_python_edit(
            PIECE,
            &payload(
                PIECE,
                &[
                    ("breath = 0.25", "breath = 0.5"),
                    ("bass = ['A2', 'E2']", "bass = ['A2', 'D2']"),
                ],
            ),
        )
        .unwrap();
        assert_eq!(
            result,
            PIECE
                .replace("breath = 0.25", "breath = 0.5")
                .replace("bass = ['A2', 'E2']", "bass = ['A2', 'D2']")
        );
        assert!(
            apply_python_edit("abcdef", &payload("abcdef", &[("abc", "x"), ("bcd", "y")]))
                .unwrap_err()
                .contains("overlap")
        );
    }

    #[test]
    fn no_op_empty_and_excessive_edits_are_rejected() {
        assert!(apply_python_edit(PIECE, &payload(PIECE, &[("breath", "breath")])).is_err());
        assert!(apply_python_edit(PIECE, &payload(PIECE, &[("", "x")])).is_err());
        assert!(apply_python_edit(PIECE, &payload(PIECE, &[])).is_err());
        assert!(apply_python_edit(PIECE, &payload(PIECE, &[("breath", "x"); 9])).is_err());
        let oversized = "x".repeat(MAX_BYTES + 1);
        assert!(apply_python_edit(&oversized, "{}").is_err());
        assert!(apply_python_edit(PIECE, &oversized).is_err());
        let large_source = format!("{}!", "x".repeat(MAX_BYTES - 1));
        assert!(apply_python_edit(&large_source, &payload(&large_source, &[("!", "!!")])).is_err());
    }
}
