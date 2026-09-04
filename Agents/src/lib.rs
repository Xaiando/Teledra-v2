use std::fmt;
use std::str::FromStr;

pub const OFFICIAL_LINKS: &str = include_str!("../social_links.md");
pub const KINGDOM_GOALS: &str = include_str!("../kingdom_goals.md");

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum AgentId {
    #[default]
    Queen,
    Organist,
    Archivist,
    Alchemist,
    Malthus,
    Orator,
    Scribe,
    Artist,
    Diplomat,
    Treasurer,
    Wizard,
}

impl AgentId {
    pub const ALL: [Self; 11] = [
        Self::Queen,
        Self::Organist,
        Self::Archivist,
        Self::Alchemist,
        Self::Malthus,
        Self::Orator,
        Self::Scribe,
        Self::Artist,
        Self::Diplomat,
        Self::Treasurer,
        Self::Wizard,
    ];

    pub const fn slug(self) -> &'static str {
        match self {
            Self::Queen => "queen",
            Self::Organist => "organist",
            Self::Archivist => "archivist",
            Self::Alchemist => "alchemist",
            Self::Malthus => "malthus",
            Self::Orator => "orator",
            Self::Scribe => "scribe",
            Self::Artist => "artist",
            Self::Diplomat => "diplomat",
            Self::Treasurer => "treasurer",
            Self::Wizard => "wizard",
        }
    }

    pub const fn title(self) -> &'static str {
        match self {
            Self::Queen => "Teledra, Sovereign of the Court",
            Self::Organist => "The Organist",
            Self::Archivist => "The Archivist",
            Self::Alchemist => "The Alchemist",
            Self::Malthus => "Malthus",
            Self::Orator => "The Orator",
            Self::Scribe => "The Scribe",
            Self::Artist => "The Artist",
            Self::Diplomat => "The Diplomat",
            Self::Treasurer => "The Treasurer",
            Self::Wizard => "The Wizard",
        }
    }

    pub const fn profile(self) -> &'static str {
        match self {
            Self::Queen => include_str!("../profiles/queen.md"),
            Self::Organist => include_str!("../profiles/organist.md"),
            Self::Archivist => include_str!("../profiles/archivist.md"),
            Self::Alchemist => include_str!("../profiles/alchemist.md"),
            Self::Malthus => include_str!("../profiles/malthus.md"),
            Self::Orator => include_str!("../profiles/orator.md"),
            Self::Scribe => include_str!("../profiles/scribe.md"),
            Self::Artist => include_str!("../profiles/artist.md"),
            Self::Diplomat => include_str!("../profiles/diplomat.md"),
            Self::Treasurer => include_str!("../profiles/treasurer.md"),
            Self::Wizard => include_str!("../profiles/wizard.md"),
        }
    }

    pub const fn supplemental_context(self) -> &'static str {
        match self {
            Self::Queen | Self::Orator | Self::Diplomat => OFFICIAL_LINKS,
            _ => "",
        }
    }
}

impl fmt::Display for AgentId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.slug())
    }
}

impl FromStr for AgentId {
    type Err = UnknownAgent;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let normalized = value.trim().to_ascii_lowercase();
        Self::ALL
            .into_iter()
            .find(|agent| agent.slug() == normalized)
            .ok_or_else(|| UnknownAgent(value.trim().to_owned()))
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct UnknownAgent(String);

impl fmt::Display for UnknownAgent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "unknown court member '{}'", self.0)
    }
}

impl std::error::Error for UnknownAgent {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_agent_round_trips_through_its_slug() {
        for agent in AgentId::ALL {
            assert_eq!(agent.slug().parse(), Ok(agent));
            assert!(!agent.profile().trim().is_empty());
        }
    }

    #[test]
    fn parsing_is_case_insensitive_and_trimmed() {
        assert_eq!("  QuEeN ".parse(), Ok(AgentId::Queen));
    }

    #[test]
    fn kingdom_goals_remain_a_concrete_operator_contract() {
        assert!(KINGDOM_GOALS.contains("Produce artifacts"));
        assert!(KINGDOM_GOALS.contains("operator approval"));
    }
}
