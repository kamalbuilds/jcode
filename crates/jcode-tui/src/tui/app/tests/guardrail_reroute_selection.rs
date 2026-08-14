// Selection rules for the guardrail reroute offer.
//
// A guardrail stop is a vendor policy decision, so the offer has to be able to
// leave the vendor. These cover the three ways that happens: a configured
// candidate wins, the refusing model is never re-offered, and an unlisted
// provider is still reachable through the cross-provider fallback.

#[cfg(test)]
mod guardrail_reroute_selection {
    use crate::tui::app::App;

    fn route(model: &str, provider: &str, api_method: &str) -> crate::provider::ModelRoute {
        crate::provider::ModelRoute {
            model: model.to_string(),
            provider: provider.to_string(),
            api_method: api_method.to_string(),
            available: true,
            detail: String::new(),
            cheapness: None,
        }
    }

    fn candidates() -> Vec<String> {
        vec![
            "claude-opus-4-8".to_string(),
            "gpt-5.6-sol".to_string(),
            "gemini-3-pro".to_string(),
        ]
    }

    /// The first configured candidate with an available route wins, and within
    /// that model the native OAuth route beats the aggregator route.
    #[test]
    fn prefers_first_candidate_and_native_auth() {
        let routes = vec![
            route("gpt-5.6-sol", "OpenAI", "openai-api-key"),
            route("claude-opus-4-8", "OpenRouter", "openrouter"),
            route("claude-opus-4-8", "Anthropic", "claude-oauth"),
        ];

        let picked = App::pick_guardrail_reroute_route(
            &routes,
            "claude-opus-5",
            "Anthropic",
            &candidates(),
            true,
        )
        .expect("a candidate is available");

        assert_eq!(picked.model, "claude-opus-4-8");
        assert_eq!(picked.api_method, "claude-oauth");
    }

    /// Rerouting to the model that just refused is pointless, so a refusal on
    /// the first candidate falls through to the next one - across vendors.
    #[test]
    fn skips_the_refusing_model_and_crosses_vendors() {
        let routes = vec![
            route("claude-opus-4-8", "Anthropic", "claude-oauth"),
            route("gpt-5.6-sol", "OpenAI", "openai-api-key"),
        ];

        let picked = App::pick_guardrail_reroute_route(
            &routes,
            "claude-opus-4-8-20260201",
            "Anthropic",
            &candidates(),
            true,
        )
        .expect("the next candidate is available");

        assert_eq!(picked.model, "gpt-5.6-sol");
    }

    /// With no configured candidate reachable, the fallback still leaves the
    /// refusing provider and prefers a frontier model over a cheap one.
    #[test]
    fn cross_provider_fallback_picks_strongest_other_provider() {
        let routes = vec![
            route("claude-sonnet-5", "Anthropic", "claude-oauth"),
            route("google/gemini-3.7-flash", "OpenRouter", "openrouter"),
            route("x-ai/grok-4.6-pro", "OpenRouter", "openrouter"),
        ];

        let picked = App::pick_guardrail_reroute_route(
            &routes,
            "claude-opus-5",
            "Anthropic",
            &candidates(),
            true,
        )
        .expect("cross-provider fallback applies");

        assert_eq!(picked.model, "x-ai/grok-4.6-pro");
        assert_eq!(picked.provider, "OpenRouter");
    }

    /// Cross-provider off restricts the offer to the configured list, so an
    /// install that wants Anthropic-only reroutes keeps that behavior.
    #[test]
    fn cross_provider_disabled_returns_none() {
        let routes = vec![
            route("claude-sonnet-5", "Anthropic", "claude-oauth"),
            route("x-ai/grok-4.6-pro", "OpenRouter", "openrouter"),
        ];

        assert!(
            App::pick_guardrail_reroute_route(
                &routes,
                "claude-opus-5",
                "Anthropic",
                &candidates(),
                false,
            )
            .is_none()
        );
    }

    /// Unavailable routes are never offered, even when the model id matches a
    /// configured candidate.
    #[test]
    fn ignores_unavailable_routes() {
        let mut unavailable = route("claude-opus-4-8", "Anthropic", "claude-oauth");
        unavailable.available = false;
        let routes = vec![unavailable, route("gpt-5.6-sol", "OpenAI", "openai-api-key")];

        let picked =
            App::pick_guardrail_reroute_route(&routes, "claude-opus-5", "Anthropic", &candidates(), true)
                .expect("the available candidate is offered");

        assert_eq!(picked.model, "gpt-5.6-sol");
    }
}
