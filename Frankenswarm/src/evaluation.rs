//! Bounded evaluation runner and regression-gated winner selection.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::Duration;

use regex::Regex;
use reqwest::Url;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EvalTask {
    pub id: String,
    pub prompt: String,
    pub expected_output: String,
    pub scoring_method: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TaskResult {
    pub id: String,
    pub category: String,
    pub score: f64,
    pub response: String,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EvaluationReport {
    pub model: String,
    pub endpoint: String,
    pub suite_sha256: String,
    pub score: f64,
    pub category_scores: BTreeMap<String, f64>,
    pub results: Vec<TaskResult>,
}

#[derive(Debug, Error)]
pub enum EvaluationError {
    #[error("evaluation I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("evaluation JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("evaluation HTTP failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("evaluation endpoint must be loopback HTTP: {0}")]
    Endpoint(String),
    #[error("invalid evaluation task: {0}")]
    Invalid(String),
    #[error("no eligible evaluation reports")]
    NoWinner,
}

pub fn load_suite(directory: &Path) -> Result<Vec<(String, EvalTask)>, EvaluationError> {
    let mut paths = fs::read_dir(directory)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("jsonl"))
        })
        .collect::<Vec<_>>();
    paths.sort();
    let mut suite = Vec::new();
    for path in paths {
        let category = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        for line in fs::read_to_string(&path)?
            .lines()
            .filter(|line| !line.trim().is_empty())
        {
            let task: EvalTask = serde_json::from_str(line)?;
            validate_task(&task)?;
            suite.push((category.clone(), task));
            if suite.len() > 10_000 {
                return Err(EvaluationError::Invalid(
                    "suite exceeds 10,000 tasks".into(),
                ));
            }
        }
    }
    if suite.is_empty() {
        return Err(EvaluationError::Invalid("suite is empty".into()));
    }
    Ok(suite)
}

pub fn run(
    directory: &Path,
    endpoint: &str,
    model: &str,
) -> Result<EvaluationReport, EvaluationError> {
    let endpoint = loopback_endpoint(endpoint)?;
    let suite = load_suite(directory)?;
    let suite_sha256 = format!("{:x}", Sha256::digest(serde_json::to_vec(&suite)?));
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(120))
        .build()?;
    let mut results = Vec::new();
    for (category, task) in suite {
        let response = client
            .post(
                endpoint
                    .join("v1/chat/completions")
                    .map_err(|error| EvaluationError::Endpoint(error.to_string()))?,
            )
            .json(&serde_json::json!({
                "model": model, "temperature": 0, "max_tokens": 2048,
                "messages": [{"role":"user","content":task.prompt}]
            }))
            .send();
        match response {
            Ok(response) => {
                let value: Value = response.error_for_status()?.json()?;
                let text = value
                    .pointer("/choices/0/message/content")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .chars()
                    .take(32_000)
                    .collect::<String>();
                let score = score(&task, &text)?;
                results.push(TaskResult {
                    id: task.id,
                    category,
                    score,
                    response: text,
                    error: None,
                });
            }
            Err(error) => results.push(TaskResult {
                id: task.id,
                category,
                score: 0.0,
                response: String::new(),
                error: Some(error.to_string()),
            }),
        }
    }
    let category_scores = aggregate(&results);
    let score = results.iter().map(|result| result.score).sum::<f64>() / results.len() as f64;
    Ok(EvaluationReport {
        model: model.into(),
        endpoint: endpoint.to_string(),
        suite_sha256,
        score,
        category_scores,
        results,
    })
}

pub fn write_report(report: &EvaluationReport, path: &Path) -> Result<(), EvaluationError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_string_pretty(report)? + "\n")?;
    Ok(())
}

pub fn select_winner(
    directory: &Path,
    minimum_category_score: f64,
) -> Result<EvaluationReport, EvaluationError> {
    let mut reports = fs::read_dir(directory)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
        })
        .filter_map(|path| fs::read_to_string(path).ok())
        .filter_map(|text| serde_json::from_str::<EvaluationReport>(&text).ok())
        .filter(|report| {
            report
                .category_scores
                .values()
                .all(|score| *score >= minimum_category_score)
        })
        .collect::<Vec<_>>();
    reports.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.model.cmp(&right.model))
    });
    reports.into_iter().next().ok_or(EvaluationError::NoWinner)
}

fn loopback_endpoint(endpoint: &str) -> Result<Url, EvaluationError> {
    let url = Url::parse(endpoint).map_err(|error| EvaluationError::Endpoint(error.to_string()))?;
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "::1"));
    if url.scheme() != "http" || !loopback || url.username() != "" || url.password().is_some() {
        return Err(EvaluationError::Endpoint(endpoint.into()));
    }
    Ok(url)
}

fn validate_task(task: &EvalTask) -> Result<(), EvaluationError> {
    if task.id.trim().is_empty()
        || task.prompt.trim().is_empty()
        || task.prompt.len() > 64 * 1024
        || task.expected_output.len() > 16 * 1024
        || !matches!(
            task.scoring_method.as_str(),
            "regex" | "exact_match" | "contains"
        )
    {
        return Err(EvaluationError::Invalid(task.id.clone()));
    }
    if task.scoring_method == "regex" {
        Regex::new(&task.expected_output)
            .map_err(|error| EvaluationError::Invalid(error.to_string()))?;
    }
    Ok(())
}

fn score(task: &EvalTask, response: &str) -> Result<f64, EvaluationError> {
    let passed = match task.scoring_method.as_str() {
        "exact_match" => response.trim() == task.expected_output.trim(),
        "contains" => response
            .to_ascii_lowercase()
            .contains(&task.expected_output.to_ascii_lowercase()),
        "regex" => Regex::new(&task.expected_output)
            .map_err(|error| EvaluationError::Invalid(error.to_string()))?
            .is_match(response),
        _ => false,
    };
    Ok(if passed { 1.0 } else { 0.0 })
}

fn aggregate(results: &[TaskResult]) -> BTreeMap<String, f64> {
    let mut totals: BTreeMap<String, (f64, usize)> = BTreeMap::new();
    for result in results {
        let entry = totals.entry(result.category.clone()).or_default();
        entry.0 += result.score;
        entry.1 += 1;
    }
    totals
        .into_iter()
        .map(|(category, (score, count))| (category, score / count as f64))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_endpoints_are_rejected() {
        assert!(loopback_endpoint("https://example.com").is_err());
        assert!(loopback_endpoint("http://127.0.0.1:8000").is_ok());
    }

    #[test]
    fn exact_match_is_actually_exact() {
        let task = EvalTask {
            id: "x".into(),
            prompt: "p".into(),
            expected_output: "yes".into(),
            scoring_method: "exact_match".into(),
        };
        assert_eq!(score(&task, "yes").unwrap(), 1.0);
        assert_eq!(score(&task, "yes indeed").unwrap(), 0.0);
    }
}
