use super::*;

fn models() -> Vec<String> {
    [
        "claude-opus-5",
        "claude-sonnet-4-6",
        "claude-haiku-4-5",
        "claude-haiku-4-5-20251001",
        "gpt-5.5",
        "gpt-5-mini",
        "gemini-3-pro",
        "gemini-3-flash",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

#[test]
fn tier_classification_covers_each_family() {
    assert_eq!(model_tier("claude-haiku-4-5"), ModelTier::Small);
    assert_eq!(model_tier("gpt-5-mini"), ModelTier::Small);
    assert_eq!(model_tier("gemini-3-flash"), ModelTier::Small);
    assert_eq!(model_tier("claude-sonnet-4-6"), ModelTier::Medium);
    assert_eq!(model_tier("claude-opus-5"), ModelTier::Large);
    assert_eq!(model_tier("gpt-5.5"), ModelTier::Large);
    assert_eq!(model_tier("gemini-3-pro"), ModelTier::Large);
}

#[test]
fn unknown_model_is_assumed_large() {
    // Conservative direction: an unknown coordinator must not be treated as
    // cheap, or the never-escalate cap would wrongly block all routing.
    assert_eq!(model_tier("some-new-frontier-model"), ModelTier::Large);
}

#[test]
fn tier_words_match_tokens_not_substrings() {
    // Regression: `gemini` contains `mini`. Substring matching classified
    // Gemini Pro as Small, which would route every heavy task to a model the
    // coordinator considered cheap.
    assert_eq!(model_tier("gemini-3-pro"), ModelTier::Large);
    assert_eq!(model_tier("gemini-3-flash"), ModelTier::Small);
    // Same shape of trap: `sonnet` must not be found inside an unrelated id.
    assert_eq!(model_tier("sonnetesque-9"), ModelTier::Large);
}

#[test]
fn family_extraction_strips_hosting_prefixes() {
    assert_eq!(model_family("claude-opus-5"), "claude");
    assert_eq!(model_family("us.anthropic.claude-opus-5"), "claude");
    assert_eq!(model_family("anthropic/claude-opus-5"), "claude");
    assert_eq!(model_family("gpt-5.5"), "gpt");
    assert_eq!(model_family("gemini-3-pro"), "gemini");
}

#[test]
fn mechanical_work_classifies_light() {
    assert_eq!(classify("search", "find all callers of foo"), Difficulty::Light);
    assert_eq!(classify("general", "list all TODO comments"), Difficulty::Light);
    assert_eq!(classify("summarizer", "summarize this file"), Difficulty::Light);
}

#[test]
fn reasoning_work_classifies_heavy() {
    assert_eq!(classify("architect", "lay out the module"), Difficulty::Heavy);
    assert_eq!(
        classify("general", "find the root cause of the panic"),
        Difficulty::Heavy
    );
    assert_eq!(
        classify("general", "check this for security issues"),
        Difficulty::Heavy
    );
}

#[test]
fn ordinary_work_classifies_standard() {
    assert_eq!(
        classify("general", "add a null check to the parser"),
        Difficulty::Standard
    );
    assert_eq!(classify("coder", "write a helper for this"), Difficulty::Standard);
}

#[test]
fn heavy_signal_beats_light_signal() {
    // "search for" is a Light marker and "security" is a Heavy one. Heavy must
    // win: a downgrade here would hand a security audit to a small model.
    assert_eq!(
        classify("searcher", "search for security vulnerabilities in auth"),
        Difficulty::Heavy
    );
}

#[test]
fn long_prompt_never_routes_light() {
    // A long spec is easy to misread. Length alone blocks the downgrade even
    // with a Light agent type and a Light marker present.
    let long = format!("summarize {}", "x".repeat(LIGHT_PROMPT_MAX_CHARS));
    assert_eq!(classify("summarizer", &long), Difficulty::Standard);
}

#[test]
fn light_task_routes_down_to_small() {
    let picked = select_model(Difficulty::Light, "claude-opus-5", &models());
    assert_eq!(picked.as_deref(), Some("claude-haiku-4-5"));
}

#[test]
fn standard_task_routes_down_to_medium() {
    let picked = select_model(Difficulty::Standard, "claude-opus-5", &models());
    assert_eq!(picked.as_deref(), Some("claude-sonnet-4-6"));
}

#[test]
fn heavy_task_inherits_coordinator() {
    assert_eq!(select_model(Difficulty::Heavy, "claude-opus-5", &models()), None);
}

#[test]
fn routing_never_escalates_above_coordinator() {
    // A sonnet coordinator with a Heavy task must not reach for opus. Routing
    // is a cost reduction only; escalation would be a silent spend increase.
    assert_eq!(
        select_model(Difficulty::Heavy, "claude-sonnet-4-6", &models()),
        None
    );
    // A haiku coordinator stays on haiku for everything.
    assert_eq!(
        select_model(Difficulty::Heavy, "claude-haiku-4-5", &models()),
        None
    );
    assert_eq!(
        select_model(Difficulty::Standard, "claude-haiku-4-5", &models()),
        None
    );
}

#[test]
fn routing_never_crosses_provider_families() {
    // Workers fork the coordinator's auth route, so a cross-family pick would
    // authenticate against the wrong provider and fail at request time.
    let picked = select_model(Difficulty::Light, "gpt-5.5", &models());
    assert_eq!(picked.as_deref(), Some("gpt-5-mini"));

    let picked = select_model(Difficulty::Light, "gemini-3-pro", &models());
    assert_eq!(picked.as_deref(), Some("gemini-3-flash"));
}

#[test]
fn missing_tier_in_family_inherits() {
    // Only opus and haiku available: a Standard task wants Medium, finds none
    // in-family, and must inherit rather than silently take the Small model.
    let sparse = vec!["claude-opus-5".to_string(), "claude-haiku-4-5".to_string()];
    assert_eq!(select_model(Difficulty::Standard, "claude-opus-5", &sparse), None);
}

#[test]
fn empty_model_list_inherits() {
    assert_eq!(select_model(Difficulty::Light, "claude-opus-5", &[]), None);
}

#[test]
fn selection_prefers_canonical_id_over_dated_variant() {
    // Both `claude-haiku-4-5` and `claude-haiku-4-5-20251001` are Small; the
    // canonical shorter id must win, deterministically.
    let picked = select_model(Difficulty::Light, "claude-opus-5", &models());
    assert_eq!(picked.as_deref(), Some("claude-haiku-4-5"));
}

#[test]
fn selection_is_deterministic_regardless_of_list_order() {
    let mut reversed = models();
    reversed.reverse();
    assert_eq!(
        select_model(Difficulty::Light, "claude-opus-5", &models()),
        select_model(Difficulty::Light, "claude-opus-5", &reversed)
    );
}

#[test]
fn hosted_coordinator_id_still_routes_in_family() {
    // A Bedrock-hosted coordinator must still match plain-id candidates.
    let picked = select_model(Difficulty::Light, "us.anthropic.claude-opus-5", &models());
    assert_eq!(picked.as_deref(), Some("claude-haiku-4-5"));
}

#[test]
fn route_end_to_end_downgrades_search_work() {
    let picked = route("searcher", "find all callers of foo", "claude-opus-5", &models());
    assert_eq!(picked.as_deref(), Some("claude-haiku-4-5"));
}

#[test]
fn route_end_to_end_keeps_frontier_for_audits() {
    let picked = route(
        "security-auditor",
        "audit the signing path",
        "claude-opus-5",
        &models(),
    );
    assert_eq!(picked, None);
}

/// The real Anthropic catalog, including the `[1m]` long-context variants that
/// the synthetic fixture above does not exercise.
fn production_anthropic_models() -> Vec<String> {
    [
        "claude-opus-5",
        "claude-opus-4-8",
        "claude-opus-4-6",
        "claude-opus-4-6[1m]",
        "claude-sonnet-5",
        "claude-sonnet-4-6",
        "claude-sonnet-4-6[1m]",
        "claude-haiku-4-5",
        "claude-opus-4-5",
        "claude-sonnet-4-5",
        "claude-sonnet-4-20250514",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

#[test]
fn production_catalog_routes_light_to_haiku() {
    let picked = select_model(
        Difficulty::Light,
        "claude-opus-5",
        &production_anthropic_models(),
    );
    assert_eq!(picked.as_deref(), Some("claude-haiku-4-5"));
}

#[test]
fn production_catalog_routes_standard_to_sonnet() {
    let picked = select_model(
        Difficulty::Standard,
        "claude-opus-5",
        &production_anthropic_models(),
    );
    // Any sonnet is acceptable; it must not be an opus or a haiku.
    let picked = picked.expect("a medium-tier model exists in the production catalog");
    assert!(picked.contains("sonnet"), "expected a sonnet, got {picked}");
}

#[test]
fn long_context_variants_keep_their_tier() {
    // `[1m]` is a context-window suffix, not a tier marker. Tokenizing must not
    // let it demote a frontier model.
    assert_eq!(model_tier("claude-opus-4-6[1m]"), ModelTier::Large);
    assert_eq!(model_tier("claude-sonnet-4-6[1m]"), ModelTier::Medium);
    assert_eq!(model_family("claude-opus-4-6[1m]"), "claude");
}

#[test]
fn production_catalog_never_escalates_from_each_tier() {
    // Property check across the whole real catalog: for every coordinator and
    // every difficulty, the routed model is never a stronger tier than the
    // coordinator's own. This is the invariant that keeps routing cost-safe.
    let catalog = production_anthropic_models();
    for coordinator in &catalog {
        let coordinator_tier = model_tier(coordinator);
        for difficulty in [Difficulty::Light, Difficulty::Standard, Difficulty::Heavy] {
            let Some(picked) = select_model(difficulty, coordinator, &catalog) else {
                continue;
            };
            assert!(
                model_tier(&picked) <= coordinator_tier,
                "{coordinator} ({coordinator_tier:?}) escalated to {picked} ({:?}) for {difficulty:?}",
                model_tier(&picked)
            );
            assert_eq!(
                model_family(&picked),
                model_family(coordinator),
                "{coordinator} crossed provider families to {picked}"
            );
        }
    }
}
