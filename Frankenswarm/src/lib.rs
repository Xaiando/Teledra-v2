//! Experimental multi-agent composition with explicit provenance.

use serde::{Deserialize, Serialize};

pub mod autopsy;
pub mod blackboard;
pub mod evaluation;
pub mod models;
pub mod runtime;
pub mod sandbox;
pub mod tournament;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Contribution {
    pub agent: String,
    pub text: String,
    pub confidence: f32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Composition {
    pub contributions: Vec<Contribution>,
    pub combined: String,
}

pub fn compose(mut contributions: Vec<Contribution>) -> Composition {
    contributions.retain(|item| !item.text.trim().is_empty());
    contributions.truncate(32);
    for contribution in &mut contributions {
        contribution.agent = contribution.agent.chars().take(80).collect();
        contribution.text = contribution.text.chars().take(16 * 1024).collect();
        contribution.confidence = if contribution.confidence.is_finite() {
            contribution.confidence.clamp(0.0, 1.0)
        } else {
            0.5
        };
    }
    contributions.sort_by(|left, right| {
        right
            .confidence
            .total_cmp(&left.confidence)
            .then_with(|| left.agent.cmp(&right.agent))
    });
    let combined = contributions
        .iter()
        .map(|item| {
            format!(
                "{} ({:.0}%): {}",
                item.agent,
                item.confidence.clamp(0.0, 1.0) * 100.0,
                item.text.trim()
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    Composition {
        contributions,
        combined,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composition_preserves_agent_provenance() {
        let result = compose(vec![Contribution {
            agent: "scribe".to_owned(),
            text: "Record it.".to_owned(),
            confidence: 0.8,
        }]);
        assert!(result.combined.contains("scribe"));
    }
}
