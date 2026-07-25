//! Sample-task demo: decompose one feature request into subtasks and show which
//! model *and reasoning effort* each spawned worker gets from the live router.
//!
//! Run: cargo run -p jcode-base --example routing_demo

use jcode_base::model_routing::{Difficulty, classify, model_tier, select_effort, select_model};

fn main() {
    // What a real Anthropic coordinator sees in its switchable model list.
    let available: Vec<String> = [
        "claude-opus-5",
        "claude-opus-4-8",
        "claude-sonnet-4-6",
        "claude-haiku-4-5",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();

    let coordinator = "claude-opus-5";
    // A coordinator deliberately running at the top of the ladder: the case
    // where inheriting effort verbatim is most expensive.
    let coordinator_effort = "max";
    let efforts: Vec<String> = ["none", "low", "medium", "high", "xhigh", "max"]
        .iter()
        .map(|s| s.to_string())
        .collect();

    // One realistic feature request, split into the subtasks a coordinator
    // would actually spawn.
    let subtasks: &[(&str, &str)] = &[
        (
            "searcher",
            "find all files that reference the auth token cache",
        ),
        (
            "reader",
            "read the session model persistence code and summarize it",
        ),
        ("docs", "rename the config key in the docs"),
        (
            "coder",
            "add a config flag that disables the token cache and wire it through",
        ),
        ("tester", "add a test covering the new config flag"),
        (
            "architect",
            "design the migration path for existing persisted sessions",
        ),
        (
            "security",
            "audit the token cache for credential leakage across sessions",
        ),
        (
            "debugger",
            "why does the cache miss after a model switch, find the root cause",
        ),
    ];

    println!(
        "coordinator: {coordinator} (tier {:?}, effort {coordinator_effort})\n",
        model_tier(coordinator)
    );
    println!(
        "{:<11} {:<52} {:<10} {:<26} {}",
        "AGENT", "SUBTASK", "DIFFICULTY", "WORKER MODEL", "EFFORT"
    );
    println!("{}", "-".repeat(120));

    let (mut light, mut standard, mut heavy) = (0, 0, 0);

    for (agent_type, prompt) in subtasks {
        let difficulty = classify(agent_type, prompt);
        match difficulty {
            Difficulty::Light => light += 1,
            Difficulty::Standard => standard += 1,
            Difficulty::Heavy => heavy += 1,
        }
        // `None` from either selector means "inherit unchanged".
        let worker = select_model(difficulty, coordinator, &available)
            .unwrap_or_else(|| format!("{coordinator} (inherited)"));
        let effort = select_effort(difficulty, Some(coordinator_effort), &efforts)
            .unwrap_or_else(|| format!("{coordinator_effort} (inherited)"));
        let truncated: String = prompt.chars().take(50).collect();
        let level = format!("{difficulty:?}");
        println!("{agent_type:<11} {truncated:<52} {level:<10} {worker:<26} {effort}");
    }

    println!(
        "\nlight={light} standard={standard} heavy={heavy} (of {})",
        subtasks.len()
    );
}
