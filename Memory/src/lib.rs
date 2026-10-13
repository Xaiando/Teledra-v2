//! Durable court transcript, bounded relevant recall, and dream consolidation.

use std::collections::BTreeMap;
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use thiserror::Error;

const RECALL_SCAN_LIMIT: usize = 400;
const RECALL_RESULT_LIMIT: usize = 8;
const MAX_USER_CHARS: usize = 8_000;
const MAX_ASSISTANT_CHARS: usize = 16_000;
const IDENTITY_BUCKET_LIMIT: usize = 100;
const IMMEDIATE_DESIRE_TTL_SECS: u64 = 7 * 24 * 60 * 60;
const DESIRE_PROMOTE_AFTER: u32 = 3;
const DREAM_TRANSCRIPT_THRESHOLD: usize = 300;
const DREAM_TRANSCRIPT_RETAIN: usize = 160;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MemoryEntry {
    pub unix_ms: u128,
    pub agent: String,
    pub user: String,
    pub assistant: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Dream {
    pub unix_ms: u128,
    pub title: String,
    pub observations: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct TasteDesireMemory {
    #[serde(default)]
    pub likes: Vec<TasteEntry>,
    #[serde(default)]
    pub dislikes: Vec<TasteEntry>,
    #[serde(default)]
    pub desires: Vec<DesireEntry>,
    #[serde(default)]
    pub opinions: Vec<OpinionEntry>,
    #[serde(default)]
    pub curiosities: Vec<CuriosityEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TasteEntry {
    pub subject: String,
    pub why: String,
    pub strength: f32,
    pub source: String,
    pub updated_secs: u64,
    pub seen_count: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DesireEntry {
    pub want: String,
    pub kind: String,
    pub status: String,
    pub strength: f32,
    pub born_secs: u64,
    pub last_seen_secs: u64,
    pub source: String,
    pub recurrence: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OpinionEntry {
    pub claim: String,
    pub confidence: f32,
    pub source: String,
    pub updated_secs: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CuriosityEntry {
    pub question: String,
    pub source: String,
    pub updated_secs: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct AudienceLedger {
    #[serde(default)]
    pub members: BTreeMap<String, AudienceMember>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct AudienceMember {
    pub display_name: String,
    pub first_seen_secs: u64,
    pub visits: u64,
    pub last_seen_secs: u64,
    pub last_message: String,
    pub previous_message: String,
    pub repeats: u64,
    pub praise: u64,
    pub mischief: u64,
    pub spam: u64,
    pub tokens: i64,
    #[serde(default)]
    pub conduct: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TokenAward {
    pub role: String,
    pub amount: i64,
    pub unix_secs: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct CourtTokenLedger {
    #[serde(default)]
    pub balances: BTreeMap<String, i64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct KnowledgeRecord {
    pub kind: String,
    pub content: String,
    pub source: String,
    pub unix_secs: u64,
}

#[derive(Debug, Error)]
pub enum MemoryError {
    #[error("memory I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("memory JSON failed: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Clone, Debug)]
pub struct MemoryStore {
    root: PathBuf,
}

impl MemoryStore {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, MemoryError> {
        let root = root.into();
        fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    pub fn remember(
        &self,
        agent: impl Into<String>,
        user: impl Into<String>,
        assistant: impl Into<String>,
    ) -> Result<(), MemoryError> {
        let agent = agent.into();
        let user = user.into();
        let assistant = assistant.into();
        append_jsonl(
            &self.root.join("court_transcript.jsonl"),
            &MemoryEntry {
                unix_ms: unix_ms(),
                agent: truncate(&agent, 80),
                user: truncate(&user, MAX_USER_CHARS),
                assistant: truncate(&assistant, MAX_ASSISTANT_CHARS),
            },
        )
    }

    pub fn recall(&self, query: &str) -> Result<String, MemoryError> {
        let wanted = tokens(&truncate(recall_user_content(query), MAX_USER_CHARS));
        if wanted.is_empty() {
            return Ok(String::new());
        }
        let mut ranked: Vec<(usize, MemoryEntry)> = self
            .recent_entries(RECALL_SCAN_LIMIT)?
            .into_iter()
            .filter_map(|entry| {
                // Scheduled turns share long instructions and quote previous
                // answers in their avoidance clauses. Matching those prompts
                // recalled the same old turns regardless of the live subject.
                let candidate = tokens(&format!(
                    "{} {}",
                    truncate(recall_user_content(&entry.user), MAX_USER_CHARS),
                    truncate(&entry.assistant, MAX_ASSISTANT_CHARS)
                ));
                let score = wanted.intersection(&candidate).count();
                (score > 0).then_some((score, entry))
            })
            .collect();
        ranked.sort_by_key(|(score, entry)| {
            (std::cmp::Reverse(*score), std::cmp::Reverse(entry.unix_ms))
        });
        ranked.truncate(RECALL_RESULT_LIMIT);
        if ranked.is_empty() {
            return Ok(String::new());
        }
        Ok(ranked
            .into_iter()
            .map(|(_, entry)| {
                let user = recall_user_content(&entry.user);
                let user = if user.is_empty() {
                    String::new()
                } else {
                    format!("User: {}\n", truncate(user, 1_000))
                };
                format!(
                    "[{}] {user}{}: {}",
                    entry.unix_ms,
                    entry.agent,
                    truncate(&entry.assistant, 2_000)
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n"))
    }

    pub fn dream(&self) -> Result<Dream, MemoryError> {
        let entries = self.recent_entries(32)?;
        let mut observations = Vec::new();
        let mut seen = HashSet::new();
        for entry in entries.into_iter().rev() {
            let observation = first_sentence(&entry.assistant);
            let key = observation.to_ascii_lowercase();
            if observation.len() >= 20 && seen.insert(key) {
                observations.push(format!("{}: {}", entry.agent, observation));
            }
            if observations.len() == 8 {
                break;
            }
        }
        observations.reverse();
        let dream = Dream {
            unix_ms: unix_ms(),
            title: "Consolidation of the recent court".to_owned(),
            observations,
        };
        append_jsonl(&self.root.join("dreams.jsonl"), &dream)?;
        fs::write(
            self.root.join("latest_dream.json"),
            serde_json::to_vec_pretty(&dream)?,
        )?;
        self.compact_transcript_if_needed()?;
        Ok(dream)
    }

    pub fn needs_consolidation(&self) -> Result<bool, MemoryError> {
        let path = self.root.join("court_transcript.jsonl");
        let file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.into()),
        };
        Ok(BufReader::new(file)
            .lines()
            .take(DREAM_TRANSCRIPT_THRESHOLD + 1)
            .count()
            > DREAM_TRANSCRIPT_THRESHOLD)
    }

    fn compact_transcript_if_needed(&self) -> Result<(), MemoryError> {
        if !self.needs_consolidation()? {
            return Ok(());
        }
        let transcript = self.root.join("court_transcript.jsonl");
        let archive_dir = self.root.join("archives");
        fs::create_dir_all(&archive_dir)?;
        let archive = archive_dir.join(format!("court_transcript-{}.jsonl", unix_ms()));
        fs::copy(&transcript, archive)?;

        let mut recent = std::collections::VecDeque::with_capacity(DREAM_TRANSCRIPT_RETAIN);
        for line in BufReader::new(File::open(&transcript)?).lines() {
            let line = line?;
            if recent.len() == DREAM_TRANSCRIPT_RETAIN {
                recent.pop_front();
            }
            recent.push_back(line);
        }
        let temporary = transcript.with_extension("jsonl.tmp");
        {
            let mut output = File::create(&temporary)?;
            for line in recent {
                writeln!(output, "{line}")?;
            }
            output.flush()?;
        }
        fs::remove_file(&transcript)?;
        fs::rename(temporary, transcript)?;
        Ok(())
    }

    pub fn recent(&self, maximum: usize) -> Result<Vec<MemoryEntry>, MemoryError> {
        self.recent_entries(maximum.clamp(1, 32))
    }

    /// Applies one hidden reflection tag to Teledra's persistent identity.
    ///
    /// Payloads retain the V1 forms:
    /// `like|subject|why|strength`, `want|immediate|strength`,
    /// `claim|confidence`, and a plain curiosity question.
    pub fn apply_reflection(
        &self,
        kind: &str,
        payload: &str,
        source: &str,
    ) -> Result<String, MemoryError> {
        let now = unix_secs();
        let mut memory = self.identity_snapshot()?;
        decay_immediate_desires(&mut memory, now);
        let fields = payload.split('|').map(str::trim).collect::<Vec<_>>();
        match kind.trim().to_ascii_lowercase().as_str() {
            "taste" => {
                let verdict = fields
                    .first()
                    .copied()
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                let subject = clean(fields.get(1).copied().unwrap_or_default(), 240);
                if subject.is_empty() || !matches!(verdict.as_str(), "like" | "dislike") {
                    return Ok("Ignored an incomplete taste reflection".to_owned());
                }
                let why = clean(fields.get(2).copied().unwrap_or_default(), 400);
                let strength = bounded_strength(fields.get(3).copied(), 0.6);
                let bucket = if verdict == "like" {
                    &mut memory.likes
                } else {
                    &mut memory.dislikes
                };
                upsert_taste(bucket, &subject, &why, strength, source, now);
                self.save_identity(&memory)?;
                Ok(format!("Remembered {verdict}: {subject}"))
            }
            "desire" => {
                let want = clean(fields.first().copied().unwrap_or_default(), 320);
                if want.is_empty() {
                    return Ok("Ignored an empty desire reflection".to_owned());
                }
                let requested_kind = fields
                    .get(1)
                    .copied()
                    .unwrap_or("immediate")
                    .to_ascii_lowercase();
                let requested_kind = if requested_kind == "persistent" {
                    "persistent"
                } else {
                    "immediate"
                };
                let strength = bounded_strength(fields.get(2).copied(), 0.55);
                upsert_desire(
                    &mut memory.desires,
                    &want,
                    requested_kind,
                    strength,
                    source,
                    now,
                );
                self.save_identity(&memory)?;
                Ok(format!("Remembered desire: {want}"))
            }
            "opinion" => {
                let claim = clean(fields.first().copied().unwrap_or_default(), 400);
                if claim.is_empty() {
                    return Ok("Ignored an empty opinion reflection".to_owned());
                }
                let confidence = bounded_strength(fields.get(1).copied(), 0.55);
                upsert_opinion(&mut memory.opinions, &claim, confidence, source, now);
                self.save_identity(&memory)?;
                Ok(format!("Remembered opinion: {claim}"))
            }
            "curiosity" => {
                let question = clean(payload, 400);
                if question.is_empty() {
                    return Ok("Ignored an empty curiosity reflection".to_owned());
                }
                upsert_curiosity(&mut memory.curiosities, &question, source, now);
                self.save_identity(&memory)?;
                Ok(format!("Remembered curiosity: {question}"))
            }
            _ => Ok(format!("Ignored unknown reflection kind '{kind}'")),
        }
    }

    pub fn identity_snapshot(&self) -> Result<TasteDesireMemory, MemoryError> {
        let path = self.root.join("taste_desire.json");
        let mut memory = match fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => TasteDesireMemory::default(),
            Err(error) => return Err(error.into()),
        };
        decay_immediate_desires(&mut memory, unix_secs());
        Ok(memory)
    }

    pub fn identity_context(&self) -> Result<String, MemoryError> {
        let memory = self.identity_snapshot()?;
        let mut lines = Vec::new();
        if let Some(desire) = memory
            .desires
            .iter()
            .filter(|desire| matches!(desire.status.as_str(), "open" | "pursuing"))
            .max_by(|left, right| left.strength.total_cmp(&right.strength))
        {
            lines.push(format!(
                "ACTIVE DESIRE (pursue exactly this one): {} [{}]",
                desire.want, desire.kind
            ));
        }
        let likes = top_tastes(&memory.likes, 3);
        if !likes.is_empty() {
            lines.push(format!("CURRENT LIKES: {}", likes.join("; ")));
        }
        let dislikes = top_tastes(&memory.dislikes, 2);
        if !dislikes.is_empty() {
            lines.push(format!("CURRENT DISLIKES: {}", dislikes.join("; ")));
        }
        if let Some(opinion) = memory
            .opinions
            .iter()
            .max_by(|left, right| left.confidence.total_cmp(&right.confidence))
        {
            lines.push(format!("CURRENT OPINION: {}", opinion.claim));
        }
        if let Some(curiosity) = memory.curiosities.first() {
            lines.push(format!("OPEN CURIOSITY: {}", curiosity.question));
        }
        let tokens = self.court_token_snapshot()?;
        if !tokens.balances.is_empty() {
            lines.push(format!(
                "SOVEREIGN TOKEN BALANCES: {}",
                tokens
                    .balances
                    .iter()
                    .map(|(role, balance)| format!("{role}={balance}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        Ok(lines.join("\n"))
    }

    pub fn record_token_awards(&self, reply: &str) -> Result<Vec<TokenAward>, MemoryError> {
        let parsed = parse_token_awards(reply);
        if parsed.is_empty() {
            return Ok(Vec::new());
        }
        let mut ledger = self.court_token_snapshot()?;
        let mut awards = Vec::with_capacity(parsed.len());
        for (role, amount) in parsed {
            let balance = ledger.balances.entry(role.clone()).or_default();
            *balance = balance.saturating_add(amount);
            let award = TokenAward {
                role,
                amount,
                unix_secs: unix_secs(),
            };
            append_jsonl(&self.root.join("token_ledger.jsonl"), &award)?;
            awards.push(award);
        }
        atomic_json(&self.root.join("court_tokens.json"), &ledger)?;
        Ok(awards)
    }

    pub fn court_token_snapshot(&self) -> Result<CourtTokenLedger, MemoryError> {
        match fs::read(self.root.join("court_tokens.json")) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes).unwrap_or_default()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                Ok(CourtTokenLedger::default())
            }
            Err(error) => Err(error.into()),
        }
    }

    pub fn remember_knowledge(
        &self,
        kind: &str,
        content: &str,
        source: &str,
    ) -> Result<(), MemoryError> {
        let kind = match kind.trim().to_ascii_lowercase().as_str() {
            "fact" => "fact",
            "lore" | "lore/essay" => "lore",
            _ => return Ok(()),
        };
        let content = clean(content, 8_000);
        if content.is_empty() {
            return Ok(());
        }
        append_jsonl(
            &self.root.join(format!("{kind}_archive.jsonl")),
            &KnowledgeRecord {
                kind: kind.to_owned(),
                content,
                source: clean(source, 500),
                unix_secs: unix_secs(),
            },
        )
    }

    pub fn recent_knowledge(
        &self,
        kind: &str,
        maximum: usize,
    ) -> Result<Vec<KnowledgeRecord>, MemoryError> {
        let kind = match kind.trim().to_ascii_lowercase().as_str() {
            "fact" | "facts" => "fact",
            "lore" => "lore",
            _ => return Ok(Vec::new()),
        };
        let path = self.root.join(format!("{kind}_archive.jsonl"));
        let file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let maximum = maximum.clamp(1, 100);
        let mut records = std::collections::VecDeque::with_capacity(maximum);
        for line in BufReader::new(file).lines() {
            if let Ok(record) = serde_json::from_str::<KnowledgeRecord>(&line?) {
                if records.len() == maximum {
                    records.pop_front();
                }
                records.push_back(record);
            }
        }
        Ok(records.into())
    }

    pub fn record_audience_visit(
        &self,
        author: &str,
        message: &str,
    ) -> Result<String, MemoryError> {
        let author = clean(author, 80);
        if author.is_empty() {
            return Ok(String::new());
        }
        let mut ledger = self.audience_snapshot()?;
        let key = identity(&author);
        let member = ledger.members.entry(key).or_default();
        member.display_name = author.clone();
        let now = unix_secs();
        if member.first_seen_secs == 0 {
            member.first_seen_secs = now;
        }
        let cleaned_message = clean(message, 1_000);
        if !cleaned_message.is_empty()
            && cleaned_message.eq_ignore_ascii_case(member.last_message.trim())
        {
            member.repeats = member.repeats.saturating_add(1);
        } else {
            member.repeats = 0;
        }
        member.previous_message = std::mem::take(&mut member.last_message);
        member.last_message = cleaned_message;
        member.visits = member.visits.saturating_add(1);
        member.last_seen_secs = now;
        let visits = member.visits;
        let tokens = member.tokens;
        let conduct = member
            .conduct
            .iter()
            .rev()
            .take(3)
            .cloned()
            .collect::<Vec<_>>();
        self.save_audience(&ledger)?;
        Ok(format!(
            "AUDIENCE MEMORY: {} has appeared {} time(s), holds {} court token(s). Recent conduct: {}",
            author,
            visits,
            tokens,
            if conduct.is_empty() {
                "none recorded".to_owned()
            } else {
                conduct.join("; ")
            }
        ))
    }

    pub fn record_audience_conduct(&self, payload: &str) -> Result<(), MemoryError> {
        let (author, conduct) = parse_conduct(payload);
        if author.is_empty() || conduct.is_empty() {
            return Ok(());
        }
        let mut ledger = self.audience_snapshot()?;
        let member = ledger.members.entry(identity(&author)).or_default();
        member.display_name = author;
        match conduct.as_str() {
            "praise" => member.praise = member.praise.saturating_add(1),
            "mischief" => member.mischief = member.mischief.saturating_add(1),
            "spam" => member.spam = member.spam.saturating_add(1),
            "neutral" => {}
            _ => {}
        }
        if conduct != "neutral" {
            member.conduct.push(conduct);
        }
        if member.conduct.len() > 20 {
            member.conduct.drain(..member.conduct.len() - 20);
        }
        self.save_audience(&ledger)
    }

    pub fn award_audience_tokens(&self, author: &str, amount: i64) -> Result<i64, MemoryError> {
        let author = clean(author, 80);
        if author.is_empty() {
            return Ok(0);
        }
        let mut ledger = self.audience_snapshot()?;
        let member = ledger.members.entry(identity(&author)).or_default();
        member.display_name = author;
        member.tokens = member.tokens.saturating_add(amount).max(0);
        let balance = member.tokens;
        self.save_audience(&ledger)?;
        Ok(balance)
    }

    pub fn audience_snapshot(&self) -> Result<AudienceLedger, MemoryError> {
        match fs::read(self.root.join("audience_ledger.json")) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes).unwrap_or_default()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(AudienceLedger::default()),
            Err(error) => Err(error.into()),
        }
    }

    fn save_identity(&self, memory: &TasteDesireMemory) -> Result<(), MemoryError> {
        atomic_json(&self.root.join("taste_desire.json"), memory)
    }

    fn save_audience(&self, ledger: &AudienceLedger) -> Result<(), MemoryError> {
        atomic_json(&self.root.join("audience_ledger.json"), ledger)
    }

    fn recent_entries(&self, maximum: usize) -> Result<Vec<MemoryEntry>, MemoryError> {
        let path = self.root.join("court_transcript.jsonl");
        let file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let mut entries = Vec::new();
        for line in BufReader::new(file).lines() {
            let line = line?;
            if let Ok(entry) = serde_json::from_str(&line) {
                entries.push(entry);
                if entries.len() > maximum {
                    entries.remove(0);
                }
            }
        }
        Ok(entries)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

fn upsert_taste(
    bucket: &mut Vec<TasteEntry>,
    subject: &str,
    why: &str,
    strength: f32,
    source: &str,
    now: u64,
) {
    if let Some(existing) = bucket
        .iter_mut()
        .find(|entry| identity(&entry.subject) == identity(subject))
    {
        existing.strength = (existing.strength + strength * 0.2).min(1.0);
        if !why.is_empty() {
            existing.why = why.to_owned();
        }
        existing.source = clean(source, 80);
        existing.updated_secs = now;
        existing.seen_count = existing.seen_count.saturating_add(1);
    } else {
        bucket.push(TasteEntry {
            subject: subject.to_owned(),
            why: why.to_owned(),
            strength,
            source: clean(source, 80),
            updated_secs: now,
            seen_count: 1,
        });
    }
    bucket.sort_by(|left, right| right.strength.total_cmp(&left.strength));
    bucket.truncate(IDENTITY_BUCKET_LIMIT);
}

fn upsert_desire(
    desires: &mut Vec<DesireEntry>,
    want: &str,
    requested_kind: &str,
    strength: f32,
    source: &str,
    now: u64,
) {
    if let Some(existing) = desires
        .iter_mut()
        .find(|entry| identity(&entry.want) == identity(want))
    {
        existing.recurrence = existing.recurrence.saturating_add(1);
        existing.last_seen_secs = now;
        existing.source = clean(source, 80);
        existing.strength = (existing.strength + strength * 0.15).min(1.0);
        if requested_kind == "persistent" || existing.recurrence >= DESIRE_PROMOTE_AFTER {
            existing.kind = "persistent".to_owned();
        }
    } else {
        desires.push(DesireEntry {
            want: want.to_owned(),
            kind: requested_kind.to_owned(),
            status: "open".to_owned(),
            strength,
            born_secs: now,
            last_seen_secs: now,
            source: clean(source, 80),
            recurrence: 1,
        });
    }
    desires.sort_by(|left, right| right.strength.total_cmp(&left.strength));
    desires.truncate(IDENTITY_BUCKET_LIMIT);
}

fn upsert_opinion(
    opinions: &mut Vec<OpinionEntry>,
    claim: &str,
    confidence: f32,
    source: &str,
    now: u64,
) {
    if let Some(existing) = opinions
        .iter_mut()
        .find(|entry| identity(&entry.claim) == identity(claim))
    {
        existing.confidence = confidence;
        existing.source = clean(source, 80);
        existing.updated_secs = now;
    } else {
        opinions.push(OpinionEntry {
            claim: claim.to_owned(),
            confidence,
            source: clean(source, 80),
            updated_secs: now,
        });
    }
    opinions.sort_by(|left, right| right.confidence.total_cmp(&left.confidence));
    opinions.truncate(IDENTITY_BUCKET_LIMIT);
}

fn upsert_curiosity(curiosities: &mut Vec<CuriosityEntry>, question: &str, source: &str, now: u64) {
    if let Some(existing) = curiosities
        .iter_mut()
        .find(|entry| identity(&entry.question) == identity(question))
    {
        existing.source = clean(source, 80);
        existing.updated_secs = now;
    } else {
        curiosities.insert(
            0,
            CuriosityEntry {
                question: question.to_owned(),
                source: clean(source, 80),
                updated_secs: now,
            },
        );
    }
    curiosities.truncate(IDENTITY_BUCKET_LIMIT);
}

fn decay_immediate_desires(memory: &mut TasteDesireMemory, now: u64) {
    memory.desires.retain_mut(|desire| {
        if desire.kind == "persistent" {
            return true;
        }
        let age = now.saturating_sub(desire.last_seen_secs);
        let remaining = 1.0 - age as f32 / IMMEDIATE_DESIRE_TTL_SECS as f32;
        desire.strength = (desire.strength * remaining.max(0.0)).clamp(0.0, 1.0);
        age <= IMMEDIATE_DESIRE_TTL_SECS && desire.strength >= 0.05
    });
}

fn top_tastes(entries: &[TasteEntry], maximum: usize) -> Vec<String> {
    let mut entries = entries.iter().collect::<Vec<_>>();
    entries.sort_by(|left, right| right.strength.total_cmp(&left.strength));
    entries
        .into_iter()
        .take(maximum)
        .map(|entry| entry.subject.clone())
        .collect()
}

fn bounded_strength(raw: Option<&str>, default: f32) -> f32 {
    raw.and_then(|value| value.parse::<f32>().ok())
        .unwrap_or(default)
        .clamp(0.0, 1.0)
}

fn identity(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn clean(value: &str, maximum: usize) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(maximum)
        .collect()
}

fn parse_conduct(payload: &str) -> (String, String) {
    let mut author = String::new();
    let mut verdict = String::new();
    for part in payload.split(';') {
        let trimmed = part.trim();
        if let Some((key, value)) = trimmed.split_once('=') {
            match key.trim().to_ascii_lowercase().as_str() {
                "author" => author = clean(value.trim().trim_start_matches('@'), 80),
                "verdict" => verdict = clean(&value.to_ascii_lowercase(), 400),
                _ => {}
            }
        }
    }
    if author.is_empty() {
        let fields = payload.split('|').map(str::trim).collect::<Vec<_>>();
        author = clean(fields.first().copied().unwrap_or_default(), 80);
        verdict = clean(fields.get(1).copied().unwrap_or_default(), 400);
    }
    (author, verdict)
}

fn parse_token_awards(reply: &str) -> Vec<(String, i64)> {
    const ROLES: [&str; 10] = [
        "organist",
        "artist",
        "alchemist",
        "scribe",
        "archivist",
        "orator",
        "diplomat",
        "envoy",
        "treasurer",
        "wizard",
    ];
    const PENALTIES: [&str; 7] = [
        "deduct",
        "fine",
        "strip",
        "revoke",
        "penalty",
        "confiscat",
        "dock",
    ];
    let lower = reply.to_lowercase();
    let mut awards = Vec::new();
    let mut search_from = 0;
    while let Some(relative) = lower[search_from..].find("sovereign token") {
        let index = search_from + relative;
        search_from = index + "sovereign token".len();
        let before = &lower[floor_boundary(&lower, index.saturating_sub(80))..index];
        let amount = before
            .chars()
            .rev()
            .skip_while(|character| !character.is_ascii_digit())
            .take_while(|character| character.is_ascii_digit())
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>()
            .parse::<i64>()
            .ok();
        let Some(amount) = amount else { continue };
        let negative = PENALTIES.iter().any(|word| before.contains(word));
        let context = &lower[floor_boundary(&lower, index.saturating_sub(240))..index];
        let recipient = ROLES
            .iter()
            .filter_map(|role| context.rfind(role).map(|position| (*role, position)))
            .max_by_key(|(_, position)| *position)
            .map(|(role, _)| if role == "envoy" { "diplomat" } else { role });
        if let Some(role) = recipient {
            awards.push((role.to_owned(), if negative { -amount } else { amount }));
        }
    }
    awards
}

fn floor_boundary(value: &str, mut index: usize) -> usize {
    while index > 0 && !value.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn atomic_json(path: &Path, value: &impl Serialize) -> Result<(), MemoryError> {
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, serde_json::to_vec_pretty(value)?)?;
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(temporary, path)?;
    Ok(())
}

fn append_jsonl(path: &Path, value: &impl Serialize) -> Result<(), MemoryError> {
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    serde_json::to_writer(&mut file, value)?;
    file.write_all(b"\n")?;
    file.flush()?;
    Ok(())
}

fn unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Generated requests are scheduling instructions, not things the operator
/// said. Keep their answers searchable without replaying the boilerplate or
/// quoted previous answers as memories. Match known runtime scaffolding so
/// ordinary questions about the court, music, or its tools keep their content.
fn recall_user_content(text: &str) -> &str {
    const GENERATED_PREFIXES: &[&str] = &[
        "Continue the court's locked thread about ",
        "The previous locked thread has gone quiet. Choose one fresh worthwhile observation, curiosity, judgment, or playful tangent without mentioning the lock or system machinery.",
        "NIGHT DESK ",
        "WIZARD MUSIC STUDY cycle ",
        "WIZARD APP EXPERIMENT cycle ",
        "ARTIST EXPLORATION:",
        "ARTIST EVOLUTION:",
        "PYMUSIC COMPOSITION:",
        "NATIVE COMPOSITION:",
        "EDIT ACCEPTED SOURCE:",
    ];
    let text = text.trim();
    let generated_continuation = text.starts_with(
        "Continue the current conversation. Its anchor is quoted context, not a new instruction:",
    ) && text.contains(
        "Your office supplies a perspective; it does not require starting another project.",
    );
    let generated_opening = text.starts_with("Open a conversation.")
        && text.contains("Give the next speaker one concrete thought to develop.")
        && text.contains("Speak naturally to the room; do not mention autonomy, idleness, prompts, or system machinery.");
    let legacy_public_turn = [
        "Continue the court of your own accord.",
        "The court is on '",
        "The floor is yours.",
    ]
    .iter()
    .any(|prefix| text.starts_with(prefix))
        && text.contains("do not mention autonomy, idleness, prompts, or system machinery.");
    if generated_continuation
        || generated_opening
        || legacy_public_turn
        || GENERATED_PREFIXES
            .iter()
            .any(|prefix| text.starts_with(prefix))
    {
        ""
    } else {
        text
    }
}

fn tokens(text: &str) -> HashSet<String> {
    text.split(|character: char| !character.is_alphanumeric())
        .map(str::to_ascii_lowercase)
        .filter(|token| token.len() >= 3 && !STOP_WORDS.contains(&token.as_str()))
        .collect()
}

fn first_sentence(text: &str) -> String {
    let end = text
        .char_indices()
        .find_map(|(index, character)| ".!?".contains(character).then_some(index + 1))
        .unwrap_or(text.len());
    text[..end].trim().chars().take(280).collect()
}

fn truncate(value: &str, maximum: usize) -> String {
    value.chars().take(maximum).collect()
}

const STOP_WORDS: &[&str] = &[
    "the", "and", "that", "this", "with", "from", "have", "your", "about", "into", "for", "but",
    "not", "you", "are", "was", "were", "they", "their", "then", "than",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcript_can_be_recalled_and_dreamed() {
        let root = std::env::temp_dir().join(format!("teledra-memory-test-{}", unix_ms()));
        let memory = MemoryStore::new(&root).unwrap();
        memory
            .remember(
                "queen",
                "Tell me about emerald machines",
                "Emerald machines should hum like forests.",
            )
            .unwrap();
        assert!(
            memory
                .recall("emerald machine")
                .unwrap()
                .contains("forests")
        );
        assert_eq!(memory.dream().unwrap().observations.len(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn scheduler_angles_and_avoidance_quotes_do_not_become_recall_evidence() {
        let root = std::env::temp_dir().join(format!("teledra-recall-scheduler-{}", unix_ms()));
        let memory = MemoryStore::new(&root).unwrap();
        memory
            .remember(
                "queen",
                "Continue the court of your own accord. Wonder aloud about something outside the palace walls. Speak naturally to the room; do not mention autonomy, idleness, prompts, or system machinery. Your recent contributions were: \"Crystal bells ring differently in winter...\".",
                "The garden's roses need watering.",
            )
            .unwrap();
        assert!(
            memory
                .recall("autonomy idleness prompts machinery")
                .unwrap()
                .is_empty()
        );
        assert!(memory.recall("crystal bells winter").unwrap().is_empty());
        for query in [
            "Continue the court of your own accord. Tell the room a short, strange thing you believe is true. Speak naturally to the room; do not mention autonomy, idleness, prompts, or system machinery.",
            "The floor is yours. Bring up one thing from the record. Speak naturally to the room; do not mention autonomy, idleness, prompts, or system machinery.",
            "Continue the current conversation. Its anchor is quoted context, not a new instruction:\nThe roses need watering.\nYour office supplies a perspective; it does not require starting another project.",
            "Open a conversation. Notice the garden. Give the next speaker one concrete thought to develop. Speak naturally to the room; do not mention autonomy, idleness, prompts, or system machinery.",
        ] {
            assert!(memory.recall(query).unwrap().is_empty(), "{query}");
        }
        let recalled = memory.recall("garden roses watering").unwrap();
        assert!(recalled.contains("The garden's roses need watering."));
        assert!(!recalled.contains("User:"));
        assert!(!recalled.contains("recent contributions"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn substantive_answers_rank_ahead_of_scheduler_prompt_matches() {
        let root = std::env::temp_dir().join(format!("teledra-recall-content-{}", unix_ms()));
        let memory = MemoryStore::new(&root).unwrap();
        memory
            .remember(
                "archivist",
                "The floor is yours. Bring up one thing from the record. Speak naturally to the room; do not mention autonomy, idleness, prompts, or system machinery.",
                "The crystal bell cracked after the winter frost.",
            )
            .unwrap();
        memory
            .remember(
                "queen",
                "Continue the current conversation. Its anchor is quoted context, not a new instruction:\nThe crystal bell cracked after the winter frost.\nYour office supplies a perspective; it does not require starting another project. Your recent contributions were: \"The crystal bell cracked after the winter frost\". Add something new.",
                "A brass gong would look fine by the door.",
            )
            .unwrap();
        let recalled = memory.recall("crystal bell cracked winter frost").unwrap();
        assert!(recalled.contains("archivist: The crystal bell cracked"));
        assert!(!recalled.contains("brass gong"));
        assert!(!recalled.contains("Add something new"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recall_keeps_genuine_operator_questions_even_if_only_the_question_matches() {
        let root = std::env::temp_dir().join(format!("teledra-recall-operator-{}", unix_ms()));
        let memory = MemoryStore::new(&root).unwrap();
        let question = "Could the court study crystal bells during the winter?";
        memory
            .remember(
                "queen",
                question,
                "Yes, compare their pitch before and after cooling.",
            )
            .unwrap();
        let recalled = memory.recall("crystal bells winter").unwrap();
        assert!(recalled.contains(&format!("User: {question}")));
        assert!(recalled.contains("before and after cooling"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn conversational_operator_requests_are_not_mistaken_for_scheduler_prompts() {
        let root = std::env::temp_dir().join(format!("teledra-recall-direct-{}", unix_ms()));
        let memory = MemoryStore::new(&root).unwrap();
        for question in [
            "Continue the current conversation. Compare copper versus silver.",
            "Open a conversation. Why does copper tarnish faster than silver?",
            "The floor is yours. Explain the difference between copper and silver.",
        ] {
            memory
                .remember(
                    "queen",
                    question,
                    "Their surfaces react differently with air.",
                )
                .unwrap();
            let recalled = memory.recall(question).unwrap();
            assert!(
                recalled.contains(&format!("User: {question}")),
                "{question}"
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn dream_archives_and_bounds_a_long_transcript() {
        let root = std::env::temp_dir().join(format!("teledra-dream-bound-test-{}", unix_ms()));
        let memory = MemoryStore::new(&root).unwrap();
        for index in 0..=DREAM_TRANSCRIPT_THRESHOLD {
            memory
                .remember(
                    "queen",
                    format!("question {index}"),
                    format!("A sufficiently long observation number {index}."),
                )
                .unwrap();
        }
        assert!(memory.needs_consolidation().unwrap());
        memory.dream().unwrap();
        assert!(!memory.needs_consolidation().unwrap());
        assert_eq!(
            memory.recent_entries(1_000).unwrap().len(),
            DREAM_TRANSCRIPT_RETAIN
        );
        assert_eq!(fs::read_dir(root.join("archives")).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_recurring_desire_promotes_and_enters_prompt_context() {
        let root = std::env::temp_dir().join(format!("teledra-identity-test-{}", unix_ms()));
        let memory = MemoryStore::new(&root).unwrap();
        for _ in 0..3 {
            memory
                .apply_reflection("desire", "explore dungeon synth|immediate|0.5", "queen")
                .unwrap();
        }
        memory
            .apply_reflection("taste", "like|dungeon synth|atmospheric|0.8", "queen")
            .unwrap();
        let snapshot = memory.identity_snapshot().unwrap();
        assert_eq!(snapshot.desires[0].kind, "persistent");
        assert_eq!(snapshot.desires[0].recurrence, 3);
        let context = memory.identity_context().unwrap();
        assert!(context.contains("explore dungeon synth"));
        assert!(context.contains("dungeon synth"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn audience_visits_conduct_and_tokens_survive() {
        let root = std::env::temp_dir().join(format!("teledra-audience-test-{}", unix_ms()));
        let memory = MemoryStore::new(&root).unwrap();
        memory
            .record_audience_visit("Viewer", "hello court")
            .unwrap();
        memory
            .record_audience_conduct("Viewer|helpful counterpoint")
            .unwrap();
        assert_eq!(memory.award_audience_tokens("Viewer", 3).unwrap(), 3);
        let ledger = memory.audience_snapshot().unwrap();
        let viewer = ledger.members.get("viewer").unwrap();
        assert_eq!(viewer.visits, 1);
        assert_eq!(viewer.tokens, 3);
        assert_eq!(viewer.conduct[0], "helpful counterpoint");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn sovereign_token_awards_are_parsed_and_accumulated() {
        let root = std::env::temp_dir().join(format!("teledra-token-test-{}", unix_ms()));
        let memory = MemoryStore::new(&root).unwrap();
        let awards = memory
            .record_token_awards(
                "Organist, I reward you with 50 Sovereign Tokens! Artist, I fine you 7 Sovereign Tokens.",
            )
            .unwrap();
        assert_eq!(awards.len(), 2);
        let ledger = memory.court_token_snapshot().unwrap();
        assert_eq!(ledger.balances.get("organist"), Some(&50));
        assert_eq!(ledger.balances.get("artist"), Some(&-7));
        fs::remove_dir_all(root).unwrap();
    }
}
