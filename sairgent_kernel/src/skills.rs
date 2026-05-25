use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SkillMetadataV1 {
    pub summary: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub trigger_hints: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_uri: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillUpsertRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill_id: Option<String>,
    pub name: String,
    pub raw_markdown: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub trigger_hints: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_agent_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillRecord {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub summary: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub trigger_hints: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_agent_id: Option<String>,
    pub current_version: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillVersionRecord {
    pub id: i64,
    pub skill_id: String,
    pub version: i64,
    pub raw_markdown: String,
    pub metadata: SkillMetadataV1,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentSkillBindingRecord {
    pub agent_id: String,
    pub skill_id: String,
    pub skill_name: String,
    pub skill_slug: String,
    pub summary: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub trigger_hints: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_uri: Option<String>,
    pub current_version: i64,
    pub priority: i64,
    pub binding_status: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RuntimeSkillIndexEntry {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub summary: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub trigger_hints: Vec<String>,
    pub current_version: i64,
    pub priority: i64,
    pub preselected: bool,
    pub score: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_path: Option<String>,
}

pub fn normalize_skill_metadata(request: &SkillUpsertRequest) -> SkillMetadataV1 {
    SkillMetadataV1 {
        summary: request
            .summary
            .clone()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| infer_summary(&request.raw_markdown, &request.name)),
        tags: normalize_terms(&request.tags),
        trigger_hints: normalize_terms(&request.trigger_hints),
        source_uri: request
            .source_uri
            .clone()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
    }
}

pub fn infer_summary(markdown: &str, fallback_name: &str) -> String {
    for line in markdown.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with('#') {
            continue;
        }
        return trimmed
            .trim_matches('`')
            .chars()
            .take(220)
            .collect::<String>();
    }
    fallback_name.trim().to_string()
}

pub fn slugify_name(name: &str) -> String {
    let mut slug = String::new();
    let mut last_dash = false;
    for ch in name.chars() {
        let normalized = match ch {
            'a'..='z' | '0'..='9' => Some(ch),
            'A'..='Z' => Some(ch.to_ascii_lowercase()),
            _ => None,
        };
        if let Some(ch) = normalized {
            slug.push(ch);
            last_dash = false;
        } else if !last_dash {
            slug.push('-');
            last_dash = true;
        }
    }
    slug.trim_matches('-').to_string()
}

pub fn build_runtime_skill_index(
    bindings: &[AgentSkillBindingRecord],
    role: &str,
    mode: &str,
    payload: &str,
    preselect_limit: usize,
) -> Vec<RuntimeSkillIndexEntry> {
    let preselected_ids = preselect_skill_ids(bindings, role, mode, payload, preselect_limit);
    let mut entries = bindings
        .iter()
        .map(|binding| RuntimeSkillIndexEntry {
            id: binding.skill_id.clone(),
            name: binding.skill_name.clone(),
            slug: binding.skill_slug.clone(),
            summary: binding.summary.clone(),
            tags: binding.tags.clone(),
            trigger_hints: binding.trigger_hints.clone(),
            current_version: binding.current_version,
            priority: binding.priority,
            preselected: preselected_ids.contains(&binding.skill_id),
            score: score_skill(binding, role, mode, payload),
            runtime_path: None,
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        right
            .preselected
            .cmp(&left.preselected)
            .then_with(|| right.score.cmp(&left.score))
            .then_with(|| left.priority.cmp(&right.priority))
            .then_with(|| left.name.cmp(&right.name))
    });
    entries
}

pub fn preselect_skill_ids(
    bindings: &[AgentSkillBindingRecord],
    role: &str,
    mode: &str,
    payload: &str,
    preselect_limit: usize,
) -> HashSet<String> {
    let limit = preselect_limit.max(1);
    let mut scored = bindings
        .iter()
        .map(|binding| {
            (
                binding.skill_id.clone(),
                score_skill(binding, role, mode, payload),
                binding.priority,
                binding.skill_name.clone(),
            )
        })
        .collect::<Vec<_>>();
    scored.sort_by(|left, right| {
        right
            .1
            .cmp(&left.1)
            .then_with(|| left.2.cmp(&right.2))
            .then_with(|| left.3.cmp(&right.3))
    });

    let mut selected = HashSet::new();
    for (idx, (skill_id, score, _, _)) in scored.iter().enumerate() {
        if idx >= limit {
            break;
        }
        if *score > 0 || idx < 2 {
            selected.insert(skill_id.clone());
        }
    }
    selected
}

pub fn score_skill(
    binding: &AgentSkillBindingRecord,
    role: &str,
    mode: &str,
    payload: &str,
) -> i64 {
    let role_lc = role.to_lowercase();
    let mode_lc = mode.to_lowercase();
    let payload_lc = payload.to_lowercase();
    let name_lc = binding.skill_name.to_lowercase();
    let summary_lc = binding.summary.to_lowercase();

    let mut score = 0_i64;

    if !payload_lc.is_empty() {
        for tag in &binding.tags {
            let tag_lc = tag.to_lowercase();
            if !tag_lc.is_empty() && payload_lc.contains(&tag_lc) {
                score += 6;
            }
            if tag_lc == role_lc {
                score += 2;
            }
        }

        for trigger in &binding.trigger_hints {
            let trigger_lc = trigger.to_lowercase();
            if !trigger_lc.is_empty() && payload_lc.contains(&trigger_lc) {
                score += 8;
            }
        }

        for token in payload_lc
            .split(|ch: char| !ch.is_ascii_alphanumeric())
            .filter(|token| token.len() >= 4)
        {
            if name_lc.contains(token) || summary_lc.contains(token) {
                score += 2;
            }
        }
    }

    for expected in mode_bias_terms(&mode_lc) {
        if binding
            .tags
            .iter()
            .any(|tag| tag.eq_ignore_ascii_case(expected))
            || binding
                .trigger_hints
                .iter()
                .any(|trigger| trigger.eq_ignore_ascii_case(expected))
            || name_lc.contains(expected)
            || summary_lc.contains(expected)
        {
            score += 4;
        }
    }

    score
}

fn mode_bias_terms(mode: &str) -> &'static [&'static str] {
    match mode {
        "chat_mode" => &["chat", "conversation", "briefing"],
        "execute_triage" => &["triage", "routing", "delegation", "planning"],
        "execute_synthesis" => &["review", "approval", "qa", "synthesis"],
        "execute_ideation" => &["innovation", "strategy", "ideation"],
        "format_swo" => &["brief", "delegation", "spec"],
        _ => &[],
    }
}

fn normalize_terms(values: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();
    for value in values {
        let trimmed = value.trim().to_lowercase();
        if trimmed.is_empty() || !seen.insert(trimmed.clone()) {
            continue;
        }
        normalized.push(trimmed);
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(
        name: &str,
        summary: &str,
        tags: &[&str],
        triggers: &[&str],
    ) -> AgentSkillBindingRecord {
        AgentSkillBindingRecord {
            agent_id: "agent-1".to_string(),
            skill_id: format!("skill-{name}"),
            skill_name: name.to_string(),
            skill_slug: slugify_name(name),
            summary: summary.to_string(),
            tags: tags.iter().map(|value| value.to_string()).collect(),
            trigger_hints: triggers.iter().map(|value| value.to_string()).collect(),
            source_uri: None,
            current_version: 1,
            priority: 100,
            binding_status: "ACTIVE".to_string(),
        }
    }

    #[test]
    fn preselects_relevant_skills_from_payload() {
        let bindings = vec![
            binding(
                "Pricing Strategy",
                "Competitive pricing guidance",
                &["pricing", "strategy"],
                &["pricing"],
            ),
            binding(
                "Hiring",
                "When to add team capacity",
                &["hiring"],
                &["hire", "staff"],
            ),
        ];

        let index = build_runtime_skill_index(
            &bindings,
            "CTO",
            "execute_triage",
            "Review pricing strategy for a new launch",
            4,
        );

        assert!(index[0].preselected);
        assert_eq!(index[0].name, "Pricing Strategy");
    }

    #[test]
    fn infers_summary_from_body_when_missing() {
        let request = SkillUpsertRequest {
            skill_id: None,
            name: "Review Skill".to_string(),
            raw_markdown: "# Review Skill\n\nUse this when doing approval passes.".to_string(),
            summary: None,
            tags: vec![],
            trigger_hints: vec![],
            source_uri: None,
            owner_agent_id: None,
        };

        let metadata = normalize_skill_metadata(&request);
        assert_eq!(metadata.summary, "Use this when doing approval passes.");
    }
}
