//! Hidden court action tags parsed away from visible/speech text.

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CourtAction {
    Research(String),
    CourtScore(String),
    Fractus {
        payload: String,
        live: bool,
    },
    Diplomacy(String),
    Suggestion(String),
    ScribeWrite {
        path: String,
        content: String,
    },
    ScribeAppend {
        path: String,
        content: String,
    },
    Delegate {
        role: String,
        instruction: String,
    },
    Reflection {
        kind: String,
        content: String,
    },
    CreativeArtifact {
        kind: String,
        content: String,
    },
    WorkshopTool(String),
    McpCall(String),
    AudienceConduct(String),
    PrivateTrace {
        kind: String,
        content: String,
    },
    MusicPatch(String),
    Fact {
        source_id: String,
        statement: String,
    },
    Lore(String),
    Topic(String),
    MoltbookComment(String),
    MoltbookUpvote(String),
    Control(CourtControl),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CourtControl {
    Unlock,
    StopBabble,
    CloseArt,
    CloseWorkshop,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParsedResponse {
    pub visible: String,
    pub actions: Vec<CourtAction>,
    /// Tag-shaped heads that matched nothing the court can execute.
    ///
    /// The court invents names — `SCOUT`, `BPM`, `FRAXUTS_ART` — and every one
    /// used to vanish without trace, so a member could appear to work for an
    /// hour while nothing it asked for ever ran. Surfacing them backstage makes
    /// the next such failure a visible line instead of an investigation.
    pub unrecognised: Vec<String>,
}

pub fn parse_response(raw: &str) -> ParsedResponse {
    let mut visible = String::with_capacity(raw.len());
    let mut actions = Vec::new();
    let mut cursor = 0;
    while let Some(relative) = raw[cursor..].find('[') {
        let start = cursor + relative;
        visible.push_str(&raw[cursor..start]);
        if let Some((end, action)) = marker_action(&raw[start..]) {
            if let Some(action) = action {
                actions.push(action);
            }
            cursor = start + end;
            continue;
        }
        let Some((end, head, payload)) = parse_tag(raw, start) else {
            visible.push('[');
            cursor = start + 1;
            continue;
        };
        if let Some(action) = into_action(head, payload) {
            actions.push(action);
        } else if let Some(action) = nearest_head(head).and_then(|corrected| {
            (!is_known_head(head))
                .then(|| into_action(corrected, payload))
                .flatten()
        }) {
            actions.push(action);
        } else if let Some((action, consumed)) =
            file_tag_with_trailing_body(head, payload, &raw[end..])
        {
            actions.push(action);
            cursor = end + consumed;
            continue;
        } else if hidden_stage_head(head) {
            // Private stage directions are omitted from speech/display.
        } else {
            visible.push_str(&raw[start..end]);
        }
        cursor = end;
    }
    visible.push_str(&raw[cursor..]);

    // The bracket is a formatting convention, not something the model reliably
    // honours. Recovering the unbracketed form has to happen here or the work
    // is silently thrown away.
    let (visible, recovered) = recover_unbracketed(&visible);
    actions.extend(recovered);

    let unrecognised = unrecognised_heads(raw, &actions);
    ParsedResponse {
        visible: sanitize_visible(&visible),
        actions,
        unrecognised,
    }
}

/// Tag-shaped heads in the reply that produced no action.
fn unrecognised_heads(raw: &str, actions: &[CourtAction]) -> Vec<String> {
    if raw.len() > 64 * 1024 {
        return Vec::new();
    }
    let mut found: Vec<String> = Vec::new();
    let bytes = raw.as_bytes();
    for (index, _) in raw.match_indices(':') {
        let head: String = raw[..index]
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_uppercase() || *c == '_')
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        if head.len() < 4 || head.len() > 24 {
            continue;
        }
        let before = index - head.len();
        if before > 0 && bytes[before - 1].is_ascii_alphanumeric() {
            continue;
        }
        if is_known_head(&head) || hidden_stage_head(&head) || found.contains(&head) {
            continue;
        }
        found.push(head);
    }
    // A head that did produce an action elsewhere in the reply is not a miss.
    let _ = actions;
    found.truncate(8);
    found
}

/// Every head the court can execute.
const ALL_HEADS: [&str; 29] = [
    "RESEARCH",
    "COURT_SCORE",
    "COURT_MUSIC_PATCH",
    "FRACTUS_ART",
    "FRACTUS_LIVE",
    "PYTHON_ART",
    "PYTHON_MUSIC",
    "PYTHON_MUSIC_EDIT",
    "STRUDEL_MUSIC",
    "DIPLOMACY",
    "SUGGESTION",
    "SCRIBE_WRITE",
    "SCRIBE_APPEND",
    "DELEGATE",
    "TASTE",
    "DESIRE",
    "OPINION",
    "CURIOSITY",
    "CURIO",
    "WORKSHOP_TOOL",
    "MCP_CALL",
    "CONDUCT",
    "THOUGHT",
    "OBSERVE",
    "FACT",
    "LORE",
    "TOPIC",
    "MOLTBOOK_COMMENT",
    "MOLTBOOK_UPVOTE",
];

/// Heads that must be spelled exactly, because acting on a guess reaches
/// outside the court.
///
/// Everything outward is approval-gated anyway, so a misread would create a
/// pending proposal rather than a post — but a proposal the operator did not
/// ask for is still noise in the approval queue, and correcting a name is not
/// worth that risk when the cost of a typo is only a missed action.
const EXACT_ONLY_HEADS: [&str; 4] = [
    "MOLTBOOK_COMMENT",
    "MOLTBOOK_UPVOTE",
    "DIPLOMACY",
    "MCP_CALL",
];

/// The known head a misspelling was reaching for, if it is unambiguous.
///
/// Observed live and repeatedly: the Artist writes `[FRAXUTS_ART: ...]`. The
/// tag is bracketed correctly and the payload is valid; only the name is wrong,
/// and every request for art was therefore discarded. A local model will not
/// stop making this mistake, so the parser accommodates it.
fn nearest_head(head: &str) -> Option<&'static str> {
    let upper = head.to_ascii_uppercase();
    if upper.len() < 8 {
        return None;
    }
    let mut best: Option<(usize, &'static str)> = None;
    let mut tied = false;
    for candidate in ALL_HEADS {
        if EXACT_ONLY_HEADS.contains(&candidate) {
            continue;
        }
        let distance = edit_distance(&upper, candidate);
        match best {
            Some((current, _)) if distance > current => {}
            Some((current, _)) if distance == current => tied = true,
            _ => {
                best = Some((distance, candidate));
                tied = false;
            }
        }
    }
    match best {
        // A unique match within two edits. Two is enough for a substitution
        // plus a transposition, which is what these misspellings are.
        Some((distance, candidate)) if distance <= 2 && !tied => Some(candidate),
        _ => None,
    }
}

/// Optimal string alignment distance, which counts an adjacent transposition
/// as one edit rather than two.
fn edit_distance(left: &str, right: &str) -> usize {
    let a: Vec<u8> = left.bytes().collect();
    let b: Vec<u8> = right.bytes().collect();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut before_previous = vec![0_usize; b.len() + 1];
    let mut current = vec![0_usize; b.len() + 1];

    for i in 1..=a.len() {
        current[0] = i;
        for j in 1..=b.len() {
            let substitution = usize::from(a[i - 1] != b[j - 1]);
            current[j] = (previous[j] + 1)
                .min(current[j - 1] + 1)
                .min(previous[j - 1] + substitution);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                current[j] = current[j].min(before_previous[j - 2] + 1);
            }
        }
        before_previous.clone_from(&previous);
        previous.clone_from(&current);
    }
    previous[b.len()]
}

/// Whether a head names something the court can execute.
fn is_known_head(head: &str) -> bool {
    let upper = head.to_ascii_uppercase();
    ALL_HEADS.contains(&upper.as_str())
}

/// Tag heads that may be recovered when the model omits the brackets.
///
/// Deliberately a closed list. Scanning for any capitalised word before a colon
/// would swallow ordinary prose like "Evidence:" or "Title:"; these are names
/// the court invented, so finding one is unambiguous.
const RECOVERABLE_HEADS: [&str; 17] = [
    "MOLTBOOK_COMMENT",
    "MOLTBOOK_UPVOTE",
    "COURT_SCORE",
    "COURT_MUSIC_PATCH",
    "FRACTUS_ART",
    "FRACTUS_LIVE",
    "PYTHON_ART",
    "PYTHON_MUSIC",
    "PYTHON_MUSIC_EDIT",
    "STRUDEL_MUSIC",
    "RESEARCH",
    "DIPLOMACY",
    "SUGGESTION",
    "DELEGATE",
    "SCRIBE_WRITE",
    "SCRIBE_APPEND",
    "MCP_CALL",
];

/// Recovers action tags written without their enclosing brackets.
///
/// Observed live: the Organist emitted `**COURT_SCORE: {"title": ...}` — the
/// name, a colon, and a complete JSON score, wrapped in markdown emphasis
/// instead of brackets. The strict parser starts scanning at `[`, so every one
/// of those compositions was discarded. The court appeared to compose
/// constantly while the music never changed.
fn recover_unbracketed(text: &str) -> (String, Vec<CourtAction>) {
    let mut actions = Vec::new();
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;

    while cursor < text.len() {
        let Some((start, head)) = next_bare_head(text, cursor) else {
            break;
        };
        let after_colon = start + head.len() + 1;
        let Some((payload, end)) = bare_payload(text, after_colon) else {
            out.push_str(&text[cursor..after_colon]);
            cursor = after_colon;
            continue;
        };
        let Some(action) = into_action(head, payload.trim()) else {
            out.push_str(&text[cursor..after_colon]);
            cursor = after_colon;
            continue;
        };
        actions.push(action);
        // Drop the emphasis the model wrapped the tag in, so the audience is
        // not left looking at a stray `**`.
        let visible_end = text[..start]
            .trim_end_matches(['*', '_', '`', '#', ' '])
            .len();
        out.push_str(&text[cursor..visible_end]);
        cursor = text[end..]
            .find(|c: char| !matches!(c, '*' | '_' | '`' | ' '))
            .map_or(text.len(), |offset| end + offset);
    }
    out.push_str(&text[cursor..]);
    (out, actions)
}

/// The next known head that is followed by a colon and stands on its own.
fn next_bare_head(text: &str, from: usize) -> Option<(usize, &'static str)> {
    let mut best: Option<(usize, &'static str)> = None;
    for head in RECOVERABLE_HEADS {
        let mut search = from;
        while let Some(relative) = text[search..].find(head) {
            let start = search + relative;
            let after = start + head.len();
            let preceded_by_word = text[..start]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
            // A longer head containing this one, such as COURT_MUSIC_PATCH
            // against COURT_SCORE, must not be split apart.
            let followed_by_word = text[after..]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
            if !preceded_by_word && !followed_by_word && text[after..].starts_with(':') {
                if best.is_none_or(|(current, _)| start < current) {
                    best = Some((start, head));
                }
                break;
            }
            search = after;
        }
    }
    best
}

/// The payload of a bare tag, and where it ends.
///
/// A JSON body is read by balancing its braces, which is what makes a whole
/// score recoverable. Anything else runs to the end of the line, because a bare
/// tag has no closing bracket to look for.
fn bare_payload(text: &str, from: usize) -> Option<(&str, usize)> {
    let offset = text[from..].find(|c: char| !c.is_whitespace())?;
    let begin = from + offset;
    if !text[begin..].starts_with('{') {
        let end = text[begin..]
            .find('\n')
            .map_or(text.len(), |index| begin + index);
        let payload = text[begin..end].trim_end_matches(['*', '_', '`', ' ', ']']);
        return (!payload.trim().is_empty()).then_some((payload, end));
    }

    let mut braces = 0_u32;
    let mut quoted = false;
    let mut escaped = false;
    for (index, character) in text[begin..].char_indices() {
        if quoted {
            match character {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => quoted = false,
                _ => {}
            }
            continue;
        }
        match character {
            '"' => quoted = true,
            '{' => braces += 1,
            '}' => {
                braces -= 1;
                if braces == 0 {
                    let end = begin + index + 1;
                    return Some((&text[begin..end], end));
                }
            }
            _ => {}
        }
    }
    None
}

/// Removes machine payloads from anything the court says aloud.
///
/// A score that fails to parse, or one salvaged without its tag, still leaves
/// its JSON in the spoken text — and it was read out to the audience: a wall of
/// `"gain": 0.8, "octave": 6, "steps": [...]` in the public feed. The audience
/// should never see a payload whether or not the runtime could use it, so this
/// strips them before anything else is decided.
fn strip_machine_payloads(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;

    loop {
        // Fenced blocks go regardless of content; nothing the court says aloud
        // belongs in a code fence.
        let fence = rest.find("```");
        // A bare object or array is only removed when it is long enough to be
        // data rather than a brace in ordinary speech.
        //
        // Every opener has to be considered, not just the first. This looked at
        // `find` alone, so one short bracket earlier in the line disabled
        // stripping for everything after it — and the court cites its sources
        // as "[FACT: S3]", which is exactly that. Twenty replies reached the
        // public feed with a full score attached because a ten-character
        // citation was the first bracket in the message.
        let bare = rest
            .char_indices()
            .filter(|(_, character)| matches!(character, '{' | '['))
            .map(|(index, _)| index)
            .find(|start| {
                balanced_span(&rest[*start..]).is_some_and(|span| span >= MACHINE_PAYLOAD_CHARS)
            });

        let next = match (fence, bare) {
            (Some(fenced), Some(literal)) => Some(fenced.min(literal)),
            (Some(fenced), None) => Some(fenced),
            (None, Some(literal)) => Some(literal),
            (None, None) => None,
        };
        let Some(start) = next else {
            out.push_str(rest);
            return out;
        };

        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        let consumed = match tail.strip_prefix("```") {
            Some(inside) => inside.find("```").map_or(tail.len(), |index| index + 6),
            None => balanced_span(tail).unwrap_or(tail.len()),
        };
        rest = &tail[consumed.min(tail.len())..];
    }
}

/// A JSON payload has to be at least this long to be treated as data.
const MACHINE_PAYLOAD_CHARS: usize = 120;

/// The length of the balanced `{...}` or `[...]` starting here, if it closes.
fn balanced_span(text: &str) -> Option<usize> {
    let opener = text.chars().next()?;
    let closer = match opener {
        '{' => '}',
        '[' => ']',
        _ => return None,
    };
    let mut depth = 0_usize;
    let mut quoted = false;
    let mut escaped = false;
    for (index, character) in text.char_indices() {
        if quoted {
            match character {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => quoted = false,
                _ => {}
            }
            continue;
        }
        match character {
            '"' => quoted = true,
            c if c == opener => depth += 1,
            c if c == closer => {
                depth -= 1;
                if depth == 0 {
                    return Some(index + character.len_utf8());
                }
            }
            _ => {}
        }
    }
    None
}

/// Whether a line is document structure rather than something a voice said.
///
/// Two independent leaks on one screen: the Organist restated a whole score as
/// a markdown field table — "**BPM:** 80", "- **Waveform:** saw" — and the
/// Artist narrated its process under "### Visual Intention:" headings. Neither
/// is a machine payload a bracket rule can find, and both read to the audience
/// as the court dumping its worksheet into the throne room.
///
/// Prose is what the court speaks. A heading is not prose, and neither is a
/// bold label with a value after it. Emphasis inside a sentence is untouched,
/// because the label has to be the whole line for this to fire.
fn is_document_structure(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.starts_with('#') {
        return true;
    }

    // A dangling JSON key line. The Organist has emitted a markdown table and
    // then key-value lines with no opening brace at all — 23 braces against 24
    // in one reply — so nothing that counts brackets can find it. The shape of
    // the line is the only thing left to recognise it by.
    if trimmed.starts_with('"') && trimmed.contains("\":") {
        return true;
    }

    // A line that is nothing but brackets is what a payload leaves behind when
    // the model never opened or never closed it.
    if !trimmed.is_empty()
        && trimmed
            .chars()
            .all(|character| matches!(character, '{' | '}' | '[' | ']' | ',' | ' '))
    {
        return true;
    }

    // Strip one list marker, so "- **Gain:** 0.7" is judged on its content.
    let marked = trimmed.starts_with("- ")
        || trimmed.starts_with("* ")
        || trimmed.split_once(". ").is_some_and(|(head, _)| {
            !head.is_empty() && head.chars().all(|character| character.is_ascii_digit())
        });
    let body = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
        .unwrap_or_else(|| {
            trimmed
                .split_once(". ")
                .filter(|(head, _)| {
                    !head.is_empty() && head.chars().all(|character| character.is_ascii_digit())
                })
                .map_or(trimmed, |(_, tail)| tail)
        })
        .trim();

    // A bold label with a colon, and nothing before it: a field, not a sentence.
    let Some(rest) = body.strip_prefix("**") else {
        return false;
    };
    let Some((label, tail)) = rest.split_once("**") else {
        return false;
    };
    let labelled = label.trim_end().ends_with(':') || tail.trim_start().starts_with(':');
    if labelled {
        // The label is not what makes a line a field; the value is. "**Gain:**
        // 0.7" is a worksheet entry, while "1. **Real-Time Generation**: Tools
        // like Pendelux now support live inputs" is a minister explaining a
        // finding. Cutting on the label alone threw away 1,300 characters of
        // the Archivist's research in one turn, which is a worse failure than
        // the leak it was written to stop.
        let value = tail.trim_start().trim_start_matches(':').trim();
        let letters = value
            .chars()
            .filter(|character| character.is_alphabetic())
            .count();
        let is_scalar = value.split_whitespace().count() <= 3
            || letters * 2 < value.chars().count()
            || (value.starts_with('[') && value.ends_with(']'));
        return is_scalar;
    }
    // "1. **Kick**" — an enumerated bold name with no sentence after it is a
    // worksheet entry. Only inside a list: a bare "**Stop.**" on its own line
    // is someone being dramatic, and that is speech.
    marked && tail.trim().is_empty()
}

fn sanitize_visible(visible: &str) -> String {
    let visible = &strip_machine_payloads(visible);
    const PRIVATE_MARKERS: [&str; 5] = [
        "[thought]",
        "[observe]",
        "[reflection]",
        "[silent reflection]",
        "[persistence]",
    ];
    visible
        .lines()
        .filter_map(|line| {
            let lower = line.to_ascii_lowercase();
            let private = PRIVATE_MARKERS
                .iter()
                .filter_map(|marker| lower.find(marker))
                .min();
            let public = private.map_or(line, |index| &line[..index]).trim_end();
            (!public.trim().is_empty() && !is_document_structure(public)).then_some(public)
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned()
}

fn parse_tag(raw: &str, start: usize) -> Option<(usize, &str, &str)> {
    let tail = &raw[start + 1..];
    let colon = tail.find(':')?;
    let head = tail[..colon].trim();
    if head.is_empty()
        || !head
            .chars()
            .all(|character| character.is_ascii_alphabetic() || character == '_')
    {
        return None;
    }
    let payload_start = start + 1 + colon + 1;
    let mut braces = 0_u32;
    let mut brackets = 0_u32;
    let mut parentheses = 0_u32;
    let mut quoted = false;
    let mut escaped = false;
    for (offset, character) in raw[payload_start..].char_indices() {
        if quoted {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                quoted = false;
            }
            continue;
        }
        match character {
            '"' => quoted = true,
            '{' => braces += 1,
            '}' => braces = braces.saturating_sub(1),
            '[' => brackets += 1,
            ']' if braces == 0 && brackets == 0 && parentheses == 0 => {
                let end = payload_start + offset;
                return Some((end + 1, head, raw[payload_start..end].trim()));
            }
            ']' => brackets = brackets.saturating_sub(1),
            '(' => parentheses += 1,
            ')' => parentheses = parentheses.saturating_sub(1),
            _ => {}
        }
    }
    None
}

fn into_action(head: &str, payload: &str) -> Option<CourtAction> {
    if payload.trim().is_empty() {
        return None;
    }
    match head.to_ascii_uppercase().as_str() {
        "RESEARCH" => Some(CourtAction::Research(payload.to_owned())),
        "COURT_SCORE" => Some(CourtAction::CourtScore(payload.to_owned())),
        "FRACTUS_ART" | "PYTHON_ART" => Some(CourtAction::Fractus {
            payload: payload.to_owned(),
            live: false,
        }),
        "FRACTUS_LIVE" => Some(CourtAction::Fractus {
            payload: payload.to_owned(),
            live: true,
        }),
        "DIPLOMACY" => Some(CourtAction::Diplomacy(payload.to_owned())),
        "SUGGESTION" => Some(CourtAction::Suggestion(payload.to_owned())),
        "SCRIBE_WRITE" => parse_file_payload(payload)
            .map(|(path, content)| CourtAction::ScribeWrite { path, content }),
        "SCRIBE_APPEND" => parse_file_payload(payload)
            .map(|(path, content)| CourtAction::ScribeAppend { path, content }),
        "DELEGATE" => parse_delegation(payload)
            .map(|(role, instruction)| CourtAction::Delegate { role, instruction }),
        "TASTE" | "DESIRE" | "OPINION" | "CURIOSITY" | "CURIO" => Some(CourtAction::Reflection {
            kind: if head.eq_ignore_ascii_case("CURIO") {
                "curiosity".to_owned()
            } else {
                head.to_ascii_lowercase()
            },
            content: payload.to_owned(),
        }),
        "PYTHON_MUSIC" => Some(CourtAction::CreativeArtifact {
            kind: "python_music".to_owned(),
            content: python_source_payload(payload),
        }),
        "STRUDEL_MUSIC" | "PYTHON_MUSIC_EDIT" => Some(CourtAction::CreativeArtifact {
            kind: head.to_ascii_lowercase(),
            content: payload.to_owned(),
        }),
        "WORKSHOP_TOOL" => Some(CourtAction::WorkshopTool(payload.to_owned())),
        "MCP_CALL" => Some(CourtAction::McpCall(payload.to_owned())),
        "CONDUCT" => Some(CourtAction::AudienceConduct(payload.to_owned())),
        "THOUGHT" | "OBSERVE" => Some(CourtAction::PrivateTrace {
            kind: head.to_ascii_lowercase(),
            content: payload.to_owned(),
        }),
        "COURT_MUSIC_PATCH" => Some(CourtAction::MusicPatch(payload.to_owned())),
        "FACT" => {
            clean_pair(payload.split_once('|')?).map(|(source_id, statement)| CourtAction::Fact {
                source_id,
                statement,
            })
        }
        "LORE" => Some(CourtAction::Lore(payload.to_owned())),
        "TOPIC" => Some(CourtAction::Topic(payload.to_owned())),
        "MOLTBOOK_COMMENT" => Some(CourtAction::MoltbookComment(payload.to_owned())),
        "MOLTBOOK_UPVOTE" => Some(CourtAction::MoltbookUpvote(payload.to_owned())),
        _ => None,
    }
}

/// Returns the consumed byte length and an optional typed action. The marker
/// forms have no colon, so they cannot use the balanced payload parser.
fn marker_action(tail: &str) -> Option<(usize, Option<CourtAction>)> {
    const HIDDEN_MARKERS: [&str; 4] = ["[DIPLOMACY]", "[FACT]", "[LORE]", "[VERIFIED]"];
    for marker in HIDDEN_MARKERS {
        if tail
            .get(..marker.len())
            .is_some_and(|candidate| candidate.eq_ignore_ascii_case(marker))
        {
            return Some((marker.len(), None));
        }
    }
    const CONTROLS: [(&str, CourtControl); 5] = [
        ("[UNLOCK]", CourtControl::Unlock),
        ("[STOP_BABBLE]", CourtControl::StopBabble),
        ("[CLOSE_ART]", CourtControl::CloseArt),
        ("[STOP_ART]", CourtControl::CloseArt),
        ("[CLOSE_WORKSHOP]", CourtControl::CloseWorkshop),
    ];
    CONTROLS.iter().find_map(|(marker, control)| {
        tail.get(..marker.len())
            .is_some_and(|candidate| candidate.eq_ignore_ascii_case(marker))
            .then_some((marker.len(), Some(CourtAction::Control(*control))))
    })
}

/// A scribe tag whose body was written after the closing bracket.
///
/// Observed live: `[SCRIBE_APPEND: ./web-crawling-strategies.md]` followed by
/// the document on the next line. The path is inside the tag and the content is
/// not, so the strict parser found a path with no content and produced nothing.
/// Every scribe tag on stream failed this way.
fn file_tag_with_trailing_body(
    head: &str,
    payload: &str,
    rest: &str,
) -> Option<(CourtAction, usize)> {
    let append = match head.to_ascii_uppercase().as_str() {
        "SCRIBE_APPEND" => true,
        "SCRIBE_WRITE" => false,
        _ => return None,
    };
    let path = payload.trim();
    // A bare path only: anything with an embedded separator was already handled
    // by the normal payload parser.
    if path.is_empty() || path.contains('|') || path.contains('\n') || path.len() > 400 {
        return None;
    }
    if !path.contains('.') && !path.contains('/') && !path.contains('\\') {
        return None;
    }

    let offset = rest.find(|c: char| !c.is_whitespace())?;
    // The body runs to the next tag, so a reply that writes a document and then
    // does something else keeps both. The tag that ends it may be indented, so
    // the boundary is found by line rather than by looking for "\n[".
    let body = &rest[offset..];
    let mut length = body.len();
    let mut scanned = 0;
    for line in body.split_inclusive('\n') {
        if scanned > 0 && line.trim_start().starts_with('[') {
            length = scanned;
            break;
        }
        scanned += line.len();
    }
    let content = body[..length].trim();
    // Short trailing text is the member still talking, not a document.
    if content.len() < 24 {
        return None;
    }

    let path = path.to_owned();
    let content = content.to_owned();
    let action = if append {
        CourtAction::ScribeAppend { path, content }
    } else {
        CourtAction::ScribeWrite { path, content }
    };
    Some((action, offset + length))
}

/// A structured source envelope has exactly one string field, `code`. Preserve
/// every other payload as legacy raw source for the normal runtime validator.
fn python_source_payload(payload: &str) -> String {
    serde_json::from_str::<serde_json::Value>(payload)
        .ok()
        .and_then(|value| {
            let object = value.as_object()?;
            if object.len() != 1 {
                return None;
            }
            object.get("code")?.as_str().map(str::to_owned)
        })
        .unwrap_or_else(|| payload.to_owned())
}

fn parse_file_payload(payload: &str) -> Option<(String, String)> {
    if let Some(split) = payload.split_once('|').or_else(|| payload.split_once('\n')) {
        return clean_pair(split);
    }
    for extension in [".jsonl", ".markdown", ".md", ".txt", ".json"] {
        if let Some(index) = payload.to_ascii_lowercase().find(extension) {
            let content_start = index + extension.len();
            let tail = &payload[content_start..];
            if let Some(relative) = tail.find(':') {
                return clean_pair((&payload[..content_start], &tail[relative + 1..]));
            }
        }
    }
    None
}

fn parse_delegation(payload: &str) -> Option<(String, String)> {
    payload
        .split_once('|')
        .or_else(|| payload.split_once(':'))
        .or_else(|| payload.split_once(char::is_whitespace))
        .and_then(clean_pair)
}

fn clean_pair((left, right): (&str, &str)) -> Option<(String, String)> {
    let left = left.trim().to_owned();
    let right = right.trim().to_owned();
    (!left.is_empty() && !right.is_empty()).then_some((left, right))
}

fn hidden_stage_head(head: &str) -> bool {
    matches!(
        head.to_ascii_lowercase().as_str(),
        "thought" | "observe" | "reflection" | "silent_reflection" | "persistence"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_citation_does_not_shield_the_payload_behind_it() {
        // Live failure: the Organist opens with "[FACT: S3]", the stripper only
        // ever looked at the first bracket, that citation was ten characters
        // long, and so the whole score behind it went to the audience. Twenty
        // replies reached the public feed this way.
        let reply = format!(
            "[FACT: S3] The court deserves grandeur.\n{{\"title\": \"A Score\", \"bpm\": 96, \"tracks\": [{}]}}",
            "{\"name\": \"Kick\", \"steps\": [0, 4, 8, 12]}, ".repeat(4)
        );
        let visible = sanitize_visible(&reply);
        assert!(
            !visible.contains("\"tracks\""),
            "the score must not be spoken: {visible}"
        );
        assert!(
            visible.contains("The court deserves grandeur."),
            "the sentence must survive: {visible}"
        );
    }

    #[test]
    fn a_scalar_field_is_not_speech_but_a_labelled_sentence_is() {
        // The Organist restates whole scores as markdown when it will not use
        // the schema, and the Archivist reports findings in the same shape. The
        // label is identical; the value is what separates a worksheet entry
        // from a minister explaining something.
        assert!(is_document_structure("   - **Gain:** 0.7"));
        assert!(is_document_structure("- **Waveform:** saw"));
        assert!(is_document_structure(
            "   - **Steps:** [0, 8, 16, 24, 32, 40, 48, 56]"
        ));
        assert!(is_document_structure("1. **Kick**"));
        assert!(is_document_structure("### Visual Intention:"));

        assert!(!is_document_structure(
            "1. **Real-Time Generation**: Tools like Pendelux now support live inputs, so a landscape can change with the light."
        ));
        assert!(!is_document_structure(
            "The Queen said **stop**, and the room went quiet."
        ));
        assert!(!is_document_structure("A plain sentence about music."));
    }

    #[test]
    fn a_payload_the_model_never_opened_is_still_not_speech() {
        // Observed: a reply carrying 23 opening braces against 24 closing ones,
        // whose JSON began with a bare key line and no object at all. Nothing
        // that counts brackets can find that, so the shape of the line has to.
        assert!(is_document_structure(
            "  \"title\": \"Serenade of the Sovereign\","
        ));
        assert!(is_document_structure("  \"tracks\":"));
        assert!(is_document_structure("}"));
        assert!(is_document_structure("  ],"));
        assert!(!is_document_structure(
            "\"Bring me something worth hearing,\" she said."
        ));
    }

    #[test]
    fn nested_json_score_is_removed_without_truncation() {
        let parsed =
            parse_response("Listen. [COURT_SCORE: {\"tracks\":[{\"steps\":[1,2]}]}] It begins.");
        assert_eq!(parsed.visible, "Listen.  It begins.");
        assert!(matches!(parsed.actions[0], CourtAction::CourtScore(_)));
    }

    #[test]
    fn markdown_links_are_not_action_tags() {
        let parsed = parse_response("Read [the source](https://example.com).");
        assert_eq!(parsed.visible, "Read [the source](https://example.com).");
        assert!(parsed.actions.is_empty());
    }

    #[test]
    fn scribe_path_and_content_are_split() {
        let parsed = parse_response("[SCRIBE_APPEND: journal.md|One useful observation.]");
        assert_eq!(
            parsed.actions,
            vec![CourtAction::ScribeAppend {
                path: "journal.md".to_owned(),
                content: "One useful observation.".to_owned()
            }]
        );
    }

    #[test]
    fn unstructured_private_stage_lines_are_not_spoken() {
        let parsed = parse_response("A public answer.\n[Thought] private chain\nStill public.");
        assert_eq!(parsed.visible, "A public answer.\nStill public.");
    }

    #[test]
    fn reflection_evidence_is_hidden_and_typed() {
        let parsed = parse_response("Interesting. [CURIOSITY: What makes it recur?]");
        assert_eq!(parsed.visible, "Interesting.");
        assert_eq!(
            parsed.actions,
            vec![CourtAction::Reflection {
                kind: "curiosity".to_owned(),
                content: "What makes it recur?".to_owned(),
            }]
        );
    }

    #[test]
    fn python_source_envelope_preserves_every_python_quote_and_bracket() {
        let code = "from teledra_synth import *\nlabel = ']'\n# A quote in a comment: \"\ndocumentation = '''single ' and \" and ]'''\nother = \"\"\"triple quotes: ] [ brackets\"\"\"\ntext = '[PYTHON_MUSIC_EDIT: only a string]'\nnotes = ['A4', 'E5']\nplay_sound(full_track, loop=True)\n";
        let envelope = serde_json::json!({"code": code}).to_string();
        let parsed = parse_response(&format!(
            "The piece is prepared. [PYTHON_MUSIC: {envelope}]"
        ));
        assert_eq!(parsed.visible, "The piece is prepared.");
        assert_eq!(
            parsed.actions,
            vec![CourtAction::CreativeArtifact {
                kind: "python_music".to_owned(),
                content: code.to_owned()
            }]
        );
        assert!(parsed.unrecognised.is_empty());
    }

    #[test]
    fn only_an_exact_source_envelope_is_decoded_and_legacy_python_still_works() {
        let code =
            "from teledra_synth import *\nnotes = ['A4', 'E5']\nplay_sound(full_track, loop=True)";
        let parsed = parse_response(&format!("[PYTHON_MUSIC: {code}]"));
        assert_eq!(
            parsed.actions,
            vec![CourtAction::CreativeArtifact {
                kind: "python_music".to_owned(),
                content: code.to_owned()
            }]
        );
        for payload in [
            r#"{"code":"pass", "unexpected":true}"#,
            r#"{"code":123}"#,
            "{invalid}",
        ] {
            let parsed = parse_response(&format!("[PYTHON_MUSIC: {payload}]"));
            assert_eq!(
                parsed.actions,
                vec![CourtAction::CreativeArtifact {
                    kind: "python_music".to_owned(),
                    content: payload.to_owned()
                }]
            );
        }
    }

    #[test]
    fn upstream_control_and_creation_tags_are_hidden_and_typed() {
        let parsed = parse_response(
            "Done. [TOPIC: modular court] [WORKSHOP_TOOL: helper.py\n```python\nprint('ok')\n```] [UNLOCK]",
        );
        assert_eq!(parsed.visible, "Done.");
        assert!(matches!(parsed.actions[0], CourtAction::Topic(_)));
        assert!(matches!(parsed.actions[1], CourtAction::WorkshopTool(_)));
        assert_eq!(
            parsed.actions[2],
            CourtAction::Control(CourtControl::Unlock)
        );
    }

    #[test]
    fn python_music_edits_preserve_exact_json_and_stay_out_of_speech() {
        let payload = serde_json::json!({
            "base": "fnv1a64:0123456789abcdef",
            "intent": "Give the answering phrase a longer breath",
            "edits": [{"old": "motif = ['A4', 'C5']\n", "new": "motif = ['A4', None, 'C5']\n"}]
        })
        .to_string();
        for raw in [
            format!("A longer breath. [PYTHON_MUSIC_EDIT: {payload}]"),
            format!("A longer breath.\n**PYTHON_MUSIC_EDIT: {payload}**"),
        ] {
            let parsed = parse_response(&raw);
            assert_eq!(parsed.visible.trim(), "A longer breath.");
            assert_eq!(
                parsed.actions,
                vec![CourtAction::CreativeArtifact {
                    kind: "python_music_edit".to_owned(),
                    content: payload.clone()
                }]
            );
            assert!(parsed.unrecognised.is_empty());
        }
    }

    #[test]
    fn original_space_delimited_delegations_and_scribe_paths_parse() {
        let parsed = parse_response(
            "[DELEGATE: ORGANIST compose a gothic waltz] [SCRIBE_APPEND: D:\\Teledra\\knowledge\\lore_archive.md: [LORE] A bell rang.]",
        );
        assert_eq!(
            parsed.actions[0],
            CourtAction::Delegate {
                role: "ORGANIST".to_owned(),
                instruction: "compose a gothic waltz".to_owned(),
            }
        );
        assert!(matches!(
            &parsed.actions[1],
            CourtAction::ScribeAppend { path, content }
                if path.ends_with("lore_archive.md") && content.starts_with("[LORE]")
        ));
    }

    /// Verbatim from the live stream: the Organist wrapped a complete score in
    /// markdown emphasis instead of brackets, and every one was discarded.
    #[test]
    fn a_markdown_wrapped_score_is_still_recovered() {
        let parsed = parse_response(
            "In celebration of the grand event, I shall compose.              **COURT_SCORE: {\"title\": \"Grand Event Overture\", \"bpm\": 140,              \"bars\": 64, \"tonic\": 5, \"mode\": \"major\", \"tracks\": []}**",
        );
        let CourtAction::CourtScore(payload) = &parsed.actions[0] else {
            panic!(
                "the score must survive without its brackets: {:?}",
                parsed.actions
            );
        };
        assert!(payload.starts_with('{') && payload.ends_with('}'));
        assert!(payload.contains("Grand Event Overture"));
        assert!(
            !parsed.visible.contains("COURT_SCORE"),
            "the tag must not be spoken aloud: {}",
            parsed.visible
        );
        assert!(parsed.visible.starts_with("In celebration"));
        assert!(
            !parsed.visible.trim_end().ends_with('*'),
            "stray emphasis remains"
        );
    }

    #[test]
    fn a_bare_tag_at_the_start_of_a_line_is_recovered() {
        let parsed = parse_response(
            "The canvas awaits.
FRACTUS_ART: --type mandala --iterations 220
And so it begins.",
        );
        assert!(matches!(
            &parsed.actions[0],
            CourtAction::Fractus { payload, live: false } if payload.contains("mandala")
        ));
        assert!(parsed.visible.contains("The canvas awaits."));
        assert!(parsed.visible.contains("And so it begins."));
        assert!(!parsed.visible.contains("FRACTUS_ART"));
    }

    #[test]
    fn bracketed_tags_are_not_recovered_twice() {
        let parsed = parse_response("[RESEARCH: local synthesis]");
        assert_eq!(parsed.actions.len(), 1, "{:?}", parsed.actions);
    }

    /// The recovery list is closed on purpose: ordinary prose that happens to
    /// put a capitalised word before a colon must stay ordinary prose.
    #[test]
    fn prose_colons_are_not_mistaken_for_tags() {
        let parsed =
            parse_response("Evidence: the coffers hold. Title: Aria. Verification: complete.");
        assert!(parsed.actions.is_empty(), "{:?}", parsed.actions);
        assert!(parsed.visible.contains("Evidence:"));
    }

    /// `COURT_SCORE` is a prefix-free match against `COURT_MUSIC_PATCH`, but a
    /// head must still never be split out of a longer surrounding word.
    #[test]
    fn a_head_inside_a_longer_word_is_ignored() {
        let parsed = parse_response("MY_RESEARCH: not a tag");
        assert!(parsed.actions.is_empty(), "{:?}", parsed.actions);
    }

    /// Verbatim shape from the live stream: the path inside the tag and the
    /// document after the closing bracket.
    #[test]
    fn a_scribe_body_written_after_the_bracket_is_still_captured() {
        let parsed = parse_response(
            "Recorded, my Queen.
[SCRIBE_APPEND: ./web-crawling-strategies.md]
             To implement focused crawling using URI queries, first define the scope.",
        );
        let CourtAction::ScribeAppend { path, content } = &parsed.actions[0] else {
            panic!("expected an append, got {:?}", parsed.actions);
        };
        assert_eq!(path, "./web-crawling-strategies.md");
        assert!(content.starts_with("To implement focused crawling"));
        assert!(parsed.visible.contains("Recorded, my Queen."));
        assert!(!parsed.visible.contains("web-crawling"));
    }

    #[test]
    fn a_scribe_tag_with_no_body_writes_nothing() {
        // Better an empty result the operator can see than a file written from
        // whatever prose happened to follow.
        let parsed = parse_response("[SCRIBE_APPEND: updates/archive.md]");
        assert!(
            !parsed
                .actions
                .iter()
                .any(|a| matches!(a, CourtAction::ScribeAppend { .. })),
            "{:?}",
            parsed.actions
        );
    }

    #[test]
    fn a_trailing_body_stops_at_the_next_tag() {
        // The closing tag may be indented, which is how the model usually
        // writes it.
        let raw = concat!(
            "[SCRIBE_WRITE: ./notes.md]
",
            "A complete finding worth preserving here.
",
            "   [RESEARCH: local synthesis]"
        );
        let parsed = parse_response(raw);
        assert!(
            matches!(
                &parsed.actions[0],
                CourtAction::ScribeWrite { content, .. }
                    if content.as_str() == "A complete finding worth preserving here."
            ),
            "{:?}",
            parsed.actions
        );
        assert!(
            matches!(&parsed.actions[1], CourtAction::Research(_)),
            "{:?}",
            parsed.actions
        );
    }

    /// Verbatim and repeated on stream: the Artist cannot spell FRACTUS.
    #[test]
    fn a_misspelled_head_still_reaches_its_action() {
        let parsed = parse_response("[FRAXUTS_ART: layers=kaleidoscope@0.45@screen@violet]");
        assert!(
            matches!(
                &parsed.actions[0],
                CourtAction::Fractus { payload, live: false } if payload.contains("kaleidoscope")
            ),
            "{:?}",
            parsed.actions
        );
    }

    #[test]
    fn outward_heads_are_never_guessed() {
        // A near miss on an outward action must not queue a proposal the
        // operator never asked for.
        let parsed = parse_response("[MOLTBOK_UPVOTE: post_id=abc]");
        assert!(
            !parsed
                .actions
                .iter()
                .any(|a| matches!(a, CourtAction::MoltbookUpvote(_))),
            "{:?}",
            parsed.actions
        );
    }

    #[test]
    fn an_unrelated_name_is_not_forced_onto_an_action() {
        let parsed = parse_response("[TELEMETRY_NOTE: nothing to do]");
        assert!(parsed.actions.is_empty(), "{:?}", parsed.actions);
        assert!(parsed.unrecognised.contains(&"TELEMETRY_NOTE".to_owned()));
    }

    /// Verbatim from the public feed: a score payload read out to the audience.
    #[test]
    fn a_score_payload_is_never_spoken_aloud() {
        let raw = concat!(
            "Let the court hear a new procession.\n",
            "{\"title\": \"Court Score\", \"bpm\": 92, \"bars\": 16, \"tracks\": [\n",
            "  {\"name\": \"Lead\", \"role\": \"lead\", \"waveform\": \"saw\",\n",
            "   \"gain\": 0.8, \"octave\": 6, \"steps\": [16, 24, 32, 40, 48, 56]},\n",
            "  {\"name\": \"Kick\", \"role\": \"kick\", \"gain\": 1.0, \"octave\": 2}\n",
            "]}\n",
            "It shall be magnificent."
        );
        let parsed = parse_response(raw);
        assert!(
            !parsed.visible.contains("\"gain\""),
            "the audience must never be read a payload: {}",
            parsed.visible
        );
        assert!(!parsed.visible.contains("steps"));
        assert!(parsed.visible.contains("Let the court hear"));
        assert!(parsed.visible.contains("It shall be magnificent."));
    }

    #[test]
    fn a_fenced_block_is_never_spoken_aloud() {
        let parsed = parse_response(
            "Here is the piece.\n```json\n{\"bpm\": 92, \"bars\": 16}\n```\nEnjoy it.",
        );
        assert!(!parsed.visible.contains("bpm"), "{}", parsed.visible);
        assert!(parsed.visible.contains("Here is the piece."));
        assert!(parsed.visible.contains("Enjoy it."));
    }

    #[test]
    fn ordinary_speech_with_braces_survives() {
        // Only payload-sized blocks are removed; a brace in prose is prose.
        let parsed = parse_response("The ledger reads {empty} and the coffers echo.");
        assert!(parsed.visible.contains("{empty}"), "{}", parsed.visible);
    }
}
