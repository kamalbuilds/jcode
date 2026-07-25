//! Difficulty-based model routing for spawned agents.
//!
//! A coordinator running a frontier model (opus, gpt-5, gemini-pro) spawns
//! workers that inherit its model verbatim. Most worker tasks - grep the repo,
//! summarize a file, rename a symbol - do not need frontier reasoning, so the
//! inherited model burns budget and context for no quality gain.
//!
//! This module maps a task to a [`Difficulty`], then picks the cheapest model
//! that clears that bar from the models actually available on the coordinator's
//! provider.
//!
//! Two invariants keep routing safe:
//!
//! 1. **Never escalate.** The selected tier is capped at the coordinator's own
//!    tier. A sonnet coordinator never spawns an opus worker, so routing can
//!    only ever reduce spend, never silently increase it.
//! 2. **Never cross families.** Spawned workers fork the coordinator's auth
//!    route (`provider_key` + `route_api_method`). Selecting a model from a
//!    different provider would keep the old auth route and fail at request
//!    time, so candidates are filtered to the coordinator's own family.
//!
//! When no candidate satisfies both invariants, routing returns `None` and the
//! caller inherits the coordinator's model. Inheriting is always correct, just
//! more expensive, so every ambiguous case degrades to today's behavior.

/// How much reasoning capability a task needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Difficulty {
    /// Mechanical, verifiable work: search, read, summarize, format, rename.
    /// A small model does this as well as a large one and much faster.
    Light,
    /// Ordinary implementation: write a function, fix a localized bug, add a
    /// test. Needs real coding ability but not deep multi-file reasoning.
    Standard,
    /// Architecture, security review, root-cause debugging, cross-cutting
    /// refactors, anything where a wrong answer is expensive to detect.
    Heavy,
}

/// Capability tier of a concrete model id, ordered cheapest to strongest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ModelTier {
    /// haiku, mini, nano, flash, lite, small
    Small,
    /// sonnet, gpt-*-mid, gemini-flash-thinking
    Medium,
    /// opus, gpt-5/6 base, gemini-pro
    Large,
}

/// Tier words that mark a cheap/fast model. Checked before [`MEDIUM_TIER_WORDS`]
/// so a compound id like `claude-haiku-4-5` resolves Small.
const SMALL_TIER_WORDS: &[&str] = &[
    "haiku", "mini", "nano", "flash", "lite", "small", "tiny", "instant", "micro",
];

/// Tier words that mark a mid-capability model.
const MEDIUM_TIER_WORDS: &[&str] = &["sonnet", "medium", "codex"];

/// Split a model id into its separator-delimited tokens.
///
/// Tier words must be matched as whole tokens, never as substrings: `gemini`
/// contains `mini`, so substring matching would classify Gemini Pro as a small
/// model and silently route every heavy task to it.
fn model_tokens(model: &str) -> impl Iterator<Item = &str> {
    model
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
}

/// Whether any separator-delimited token of `model` equals one of `words`.
fn has_tier_token(model: &str, words: &[&str]) -> bool {
    model_tokens(model).any(|token| words.contains(&token))
}

/// Classify a model id into a capability tier.
///
/// Unknown ids resolve to [`ModelTier::Large`]. That is the conservative
/// direction: an unrecognized model is assumed strong, so the never-escalate cap
/// stays permissive and an unknown coordinator never blocks routing entirely.
pub fn model_tier(model: &str) -> ModelTier {
    let id = model.to_ascii_lowercase();
    if has_tier_token(&id, SMALL_TIER_WORDS) {
        return ModelTier::Small;
    }
    if has_tier_token(&id, MEDIUM_TIER_WORDS) {
        return ModelTier::Medium;
    }
    ModelTier::Large
}

/// Extract the provider family from a model id, e.g. `claude`, `gpt`, `gemini`.
///
/// Hosted vendor prefixes (Bedrock's `us.`, `anthropic.`) and any leading
/// `vendor/` segment (OpenRouter style) are stripped first so the same logical
/// model routes identically across hosting surfaces.
pub fn model_family(model: &str) -> String {
    let mut id = model.to_ascii_lowercase();

    // OpenRouter-style `anthropic/claude-opus-5` -> `claude-opus-5`.
    if let Some((_, rest)) = id.split_once('/') {
        id = rest.to_string();
    }

    // Bedrock-style region and vendor prefixes.
    for prefix in [
        "us-gov.",
        "us.",
        "eu.",
        "apac.",
        "ap.",
        "global.",
        "anthropic.",
        "meta.",
        "mistral.",
        "amazon.",
    ] {
        if let Some(rest) = id.strip_prefix(prefix) {
            id = rest.to_string();
        }
    }

    // The family is the leading alphabetic run: `claude-opus-5` -> `claude`,
    // `gpt-5.5` -> `gpt`, `gemini-3-pro` -> `gemini`, `o3-mini` -> `o`.
    let family: String = id.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    if family.is_empty() { id } else { family }
}

/// Subagent type names that reliably indicate mechanical work.
const LIGHT_AGENT_TYPES: &[&str] = &[
    "search",
    "searcher",
    "grep",
    "find",
    "finder",
    "lookup",
    "fetch",
    "fetcher",
    "read",
    "reader",
    "summarize",
    "summarizer",
    "summary",
    "format",
    "formatter",
    "lint",
    "linter",
    "rename",
    "docs",
    "doc",
    "scout",
    "explore",
    "explorer",
    "collect",
    "collector",
    "extract",
    "extractor",
];

/// Subagent type names that indicate deep reasoning.
const HEAVY_AGENT_TYPES: &[&str] = &[
    "architect",
    "architecture",
    "design",
    "designer",
    "debug",
    "debugger",
    "security",
    "auditor",
    "audit",
    "review",
    "reviewer",
    "critic",
    "refactor",
    "planner",
    "plan",
    "judge",
    "researcher",
    "research",
];

/// Prompt phrases that force [`Difficulty::Heavy`] regardless of agent type.
///
/// These name work where a plausible-but-wrong answer is expensive: the failure
/// is silent and surfaces later as a bug, an outage, or a security hole.
const HEAVY_PROMPT_MARKERS: &[&str] = &[
    "root cause",
    "root-cause",
    "architect",
    "design a",
    "design the",
    "trade-off",
    "tradeoff",
    "security",
    "vulnerab",
    "exploit",
    "race condition",
    "deadlock",
    "memory leak",
    "why does",
    "why is",
    "refactor",
    "migrate",
    "prove",
    "verify correctness",
    "threat model",
];

/// Prompt phrases that indicate purely mechanical work.
const LIGHT_PROMPT_MARKERS: &[&str] = &[
    "list all",
    "find all",
    "grep for",
    "search for",
    "count the",
    "summarize",
    "what files",
    "which files",
    "read the",
    "fetch the",
    "extract the",
    "rename ",
    "format ",
];

/// Prompt length (characters) above which a task is never classified Light.
///
/// A long prompt carries enough specification detail that misreading it is
/// likely, and the cost of a small model misreading a long spec exceeds the
/// savings.
const LIGHT_PROMPT_MAX_CHARS: usize = 600;

/// Classify a task's difficulty from its agent type and prompt.
///
/// Ties break toward [`Difficulty::Standard`], and any Heavy signal wins over a
/// Light one. Downgrades happen only on clear evidence because a wrong downgrade
/// produces confidently wrong work, which is far more expensive than the tokens
/// it saves.
pub fn classify(agent_type: &str, prompt: &str) -> Difficulty {
    let agent = agent_type.to_ascii_lowercase();
    let text = prompt.to_ascii_lowercase();

    let heavy_agent = HEAVY_AGENT_TYPES.iter().any(|t| agent.contains(t));
    let heavy_prompt = HEAVY_PROMPT_MARKERS.iter().any(|m| text.contains(m));
    if heavy_agent || heavy_prompt {
        return Difficulty::Heavy;
    }

    let light_agent = LIGHT_AGENT_TYPES.iter().any(|t| agent.contains(t));
    let light_prompt = LIGHT_PROMPT_MARKERS.iter().any(|m| text.contains(m));
    let short_enough = prompt.chars().count() <= LIGHT_PROMPT_MAX_CHARS;

    if (light_agent || light_prompt) && short_enough {
        return Difficulty::Light;
    }

    Difficulty::Standard
}

/// The tier a difficulty level wants, before the never-escalate cap.
fn desired_tier(difficulty: Difficulty) -> ModelTier {
    match difficulty {
        Difficulty::Light => ModelTier::Small,
        Difficulty::Standard => ModelTier::Medium,
        Difficulty::Heavy => ModelTier::Large,
    }
}

/// Pick the model a spawned worker should run for `difficulty`.
///
/// Returns `None` when the coordinator's model should be inherited unchanged,
/// which is the correct fallback for every ambiguous case.
///
/// `available` is the coordinator provider's model list. Candidates are filtered
/// to the coordinator's family so the inherited auth route stays valid, and to
/// tiers at or below the coordinator's own so routing never escalates spend.
pub fn select_model(
    difficulty: Difficulty,
    coordinator_model: &str,
    available: &[String],
) -> Option<String> {
    let coordinator_tier = model_tier(coordinator_model);
    let target = desired_tier(difficulty).min(coordinator_tier);

    // Already at the target: inherit rather than swap to a same-tier sibling,
    // which would churn the model for no benefit.
    if coordinator_tier == target {
        return None;
    }

    let family = model_family(coordinator_model);
    let pick = available
        .iter()
        .filter(|candidate| model_family(candidate) == family)
        .filter(|candidate| model_tier(candidate) == target)
        // Shortest id wins: prefers the canonical `claude-haiku-4-5` over a
        // dated or hosted variant of the same model, and is deterministic.
        .min_by_key(|candidate| (candidate.len(), candidate.to_string()))?;

    if pick == coordinator_model {
        return None;
    }
    Some(pick.clone())
}

/// Convenience wrapper: classify then select in one call.
pub fn route(
    agent_type: &str,
    prompt: &str,
    coordinator_model: &str,
    available: &[String],
) -> Option<String> {
    select_model(classify(agent_type, prompt), coordinator_model, available)
}

/// Reasoning effort ladder, weakest to strongest, as accepted on the wire.
///
/// `swarm`/`swarm-deep` are UI sentinels that sit above `max`; they are ranked
/// at the top so a coordinator running one is never treated as cheap by the
/// never-escalate cap.
const EFFORT_LADDER: &[&str] = &[
    "none",
    "low",
    "medium",
    "high",
    "xhigh",
    "max",
    "swarm",
    "swarm-deep",
];

/// Position of `effort` on [`EFFORT_LADDER`], or `None` if unrecognized.
fn effort_rank(effort: &str) -> Option<usize> {
    let normalized = effort.trim().to_ascii_lowercase();
    EFFORT_LADDER.iter().position(|level| *level == normalized)
}

/// The effort a difficulty level wants, before the never-escalate cap.
///
/// Light work is mechanical and verifiable, so thinking tokens buy nothing.
/// Heavy work returns `None`, meaning "inherit": a coordinator that deliberately
/// raised its own effort must not have that decision silently overridden.
fn desired_effort(difficulty: Difficulty) -> Option<&'static str> {
    match difficulty {
        Difficulty::Light => Some("none"),
        Difficulty::Standard => Some("medium"),
        Difficulty::Heavy => None,
    }
}

/// Pick the reasoning effort a spawned worker should run for `difficulty`.
///
/// Returns `None` when the coordinator's effort should be inherited unchanged.
///
/// This is the effort half of [`select_model`] and carries the same
/// never-escalate invariant: the result is capped at the coordinator's own
/// effort, so routing can only ever reduce thinking spend. Without it a routed
/// worker gets a cheap model running at the coordinator's `max` effort, which is
/// the expensive half of the bill for mechanical work.
///
/// `available` is the effort list the worker's provider/model actually accepts
/// (`Provider::available_efforts`). A level absent from that list is stepped
/// down to the strongest supported level at or below it, so a model with a
/// shorter ladder never receives an effort it would reject.
pub fn select_effort(
    difficulty: Difficulty,
    coordinator_effort: Option<&str>,
    available: &[String],
) -> Option<String> {
    let target = desired_effort(difficulty)?;
    let target_rank = effort_rank(target)?;

    // Unknown or absent coordinator effort means the model default is in play,
    // which we cannot compare against, so inherit rather than guess.
    let coordinator_rank = effort_rank(coordinator_effort?)?;
    if coordinator_rank <= target_rank {
        return None;
    }

    // Step down to the strongest level the worker's model actually accepts,
    // never above the target and never above the coordinator.
    let pick = available
        .iter()
        .filter_map(|level| effort_rank(level).map(|rank| (rank, level)))
        .filter(|(rank, _)| *rank <= target_rank)
        .max_by_key(|(rank, _)| *rank)
        .map(|(_, level)| level.clone())?;

    if coordinator_effort.map(str::trim) == Some(pick.as_str()) {
        return None;
    }
    Some(pick)
}

#[cfg(test)]
#[path = "model_routing_tests.rs"]
mod tests;
