//! The model lane (ADR-0067 §1): the committed fixtures, run through
//! the real agent loop against a live model.
//!
//!     cargo run -p coder-eval -- [--url URL] [--model ID] [--only NAME]
//!                                [--out report.json]
//!
//! URL defaults to the local inferenced companion; a `remote:*` model id
//! routes through it to the broker like any Assistant run. The score is
//! `resolved/total` over every discovered fixture — an environment or
//! agent failure is a counted FAIL, never a skip. This lane can only
//! see LARGE moves (±9pp noise floor at this N — ADR-0067); improvement
//! claims belong to the public tier, which is not built yet.

use coder_eval::{Report, TaskReport, discover, fixtures_dir, grade, restore_tests, sanity, stage};
use forge_harness::{AgentConfig, OpenAiBackend, ToolProvider, Verifier, WorkspaceTools};
use std::sync::Arc;
use std::time::Instant;

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn main() {
    let url = arg("--url").unwrap_or_else(|| "http://127.0.0.1:7778".to_string());
    let model = arg("--model");
    let only = arg("--only");
    let out = arg("--out");

    let fixtures = match discover(&fixtures_dir()) {
        Ok(f) => f
            .into_iter()
            .filter(|f| only.as_deref().is_none_or(|o| f.name == o))
            .collect::<Vec<_>>(),
        Err(e) => {
            eprintln!("coder-eval: {e}");
            std::process::exit(2);
        }
    };
    if fixtures.is_empty() {
        eprintln!("coder-eval: no fixtures matched");
        std::process::exit(2);
    }

    let tmp = tempfile::tempdir().expect("tempdir");
    let ledger_dir = tempfile::tempdir().expect("ledger dir");
    let ledger = Arc::new(
        lisa_ledger::Ledger::open(ledger_dir.path().join("eval-ledger.db")).expect("ledger opens"),
    );

    let mut tasks = Vec::new();
    for fixture in &fixtures {
        let started = Instant::now();
        let report = run_one(fixture, tmp.path(), &url, model.as_deref(), &ledger);
        let wall_ms = started.elapsed().as_millis();
        let resolved = report.is_none();
        println!(
            "  {} {}{}",
            if resolved { "ok  " } else { "FAIL" },
            fixture.name,
            report
                .as_ref()
                .map(|f| format!(" — {f:?}"))
                .unwrap_or_default()
        );
        tasks.push(TaskReport {
            name: fixture.name.clone(),
            resolved,
            failure: report,
            wall_ms,
        });
    }

    let report = Report {
        lane: "model".into(),
        resolved: tasks.iter().filter(|t| t.resolved).count(),
        total: tasks.len(),
        tasks,
    };
    println!("{}", report.summary());
    if let Some(path) = out {
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&report).expect("serializes"),
        )
        .unwrap_or_else(|e| eprintln!("coder-eval: writing {path}: {e}"));
    }
    if report.resolved < report.total {
        std::process::exit(1);
    }
}

fn run_one(
    fixture: &coder_eval::Fixture,
    stage_root: &std::path::Path,
    url: &str,
    model: Option<&str>,
    ledger: &Arc<lisa_ledger::Ledger>,
) -> Option<coder_eval::Failure> {
    let ws = match stage(fixture, stage_root) {
        Ok(ws) => ws,
        Err(e) => return Some(coder_eval::Failure::Environment(e)),
    };
    if let Err(f) = sanity(fixture, &ws) {
        return Some(f);
    }
    let tools = match WorkspaceTools::new(&ws) {
        Ok(t) => t,
        Err(e) => return Some(coder_eval::Failure::Environment(e.to_string())),
    };
    let providers: [&dyn ToolProvider; 1] = [&tools];
    let mut backend = OpenAiBackend {
        url: url.to_string(),
        model: model.map(str::to_string),
    };
    let config = AgentConfig {
        max_turns: 24,
        // The loop's own gate is the project's tests; grading below is
        // separate and happens on the restored tree.
        verifier: Verifier::Command {
            program: "cargo".into(),
            args: vec!["test".into(), "--offline".into(), "-q".into()],
        },
        ..AgentConfig::new(Arc::clone(ledger))
    };
    let run = forge_harness::forge_agent_with_tools(
        &fixture.task.prompt,
        &ws,
        &mut backend,
        &config,
        &providers,
        &mut |_| {},
    );
    if let Err(e) = run {
        return Some(coder_eval::Failure::Agent(e.to_string()));
    }
    if let Err(e) = restore_tests(fixture, &ws) {
        return Some(coder_eval::Failure::Environment(e));
    }
    grade(fixture, &ws).err()
}
