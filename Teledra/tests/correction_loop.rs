//! The runtime asks for a correction when a payload cannot be used.
//!
//! Recovering each new malformed shape needs a new parser, and the model keeps
//! inventing shapes: brackets dropped, names misspelled, and then a score
//! written as markdown headings. Handing the parse error back is the response
//! that covers shapes nobody has anticipated, so it is worth pinning down that
//! a bad payload still reaches the runtime as an attempt rather than as prose.

use teledra_court::{CourtAction, parse_response};

/// Verbatim from the stream: a score written as markdown headings, which no
/// amount of tag recovery can turn into JSON.
const MARKDOWN_SCORE: &str = "In this sovereign court, I shall compose. \
     **COURT_SCORE:** **Title:** *Harmonious Accord* **BPM:** 160 **Bars:** 64";

#[test]
fn a_markdown_score_reaches_the_runtime_as_an_attempt() {
    let parsed = parse_response(MARKDOWN_SCORE);
    assert!(
        parsed
            .actions
            .iter()
            .any(|action| matches!(action, CourtAction::CourtScore(_))),
        "the attempt must reach the runtime to be correctable: {:?}",
        parsed.actions
    );
}

#[test]
fn a_markdown_score_does_not_parse_as_music() {
    let parsed = parse_response(MARKDOWN_SCORE);
    let Some(CourtAction::CourtScore(raw)) = parsed
        .actions
        .into_iter()
        .find(|action| matches!(action, CourtAction::CourtScore(_)))
    else {
        panic!("expected a score attempt");
    };
    assert!(
        teledra_music::Score::from_json(&raw).is_err(),
        "markdown headings are not a score; the runtime must ask for a correction"
    );
}
