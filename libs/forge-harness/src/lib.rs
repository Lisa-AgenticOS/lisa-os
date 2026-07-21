//! forge-harness — the agentic app-building loop (`docs/PLAN.md`
//! §5.12.1), walking skeleton.
//!
//! plan → edit (jailed to the project dir) → `dart analyze` → iterate,
//! against any OpenAI-compatible backend — lisa-inferenced's local coder
//! model by default, a BYO agent CLI later, both through the same tool
//! jail. Edits use *guided generation*: the backend is constrained to a
//! `{path, content}` JSON schema, so the harness never parses free-form
//! model output. Hot-reload preview + VLM self-inspection join the loop
//! next (run-controller).

pub mod jail;

use jail::Jail;
use serde::Deserialize;
use std::path::Path;
use std::process::Command;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ForgeError {
    #[error("backend: {0}")]
    Backend(String),
    #[error("jail: {0}")]
    Jail(#[from] jail::JailError),
    #[error("analyzer: {0}")]
    Analyzer(String),
    #[error("did not converge after {0} iteration(s)")]
    NoConvergence(usize),
}

/// One file edit, as the schema the backend is constrained to.
#[derive(Debug, Deserialize)]
pub struct Edit {
    pub path: String,
    pub content: String,
}

/// A completion backend. The production impl speaks OpenAI-compat with
/// guided generation; tests script it.
pub trait Backend {
    fn edit_for(&mut self, task: &str, context: &str) -> Result<Edit, ForgeError>;
}

/// OpenAI-compat backend (lisa-inferenced or any compatible endpoint).
pub struct OpenAiBackend {
    pub url: String,
    pub model: Option<String>,
}

const EDIT_SCHEMA: &str = r#"{
  "type": "object",
  "properties": {
    "path": { "type": "string", "maxLength": 200 },
    "content": { "type": "string" }
  },
  "required": ["path", "content"]
}"#;

impl Backend for OpenAiBackend {
    fn edit_for(&mut self, task: &str, context: &str) -> Result<Edit, ForgeError> {
        let schema: serde_json::Value =
            serde_json::from_str(EDIT_SCHEMA).expect("static schema parses");
        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content":
                    "You are the Lisa Forge. Reply with a JSON object {path, content}: \
                     the project-relative file path and the COMPLETE new file content."},
                {"role": "user", "content": format!("Task: {task}\n\n{context}")}
            ],
            "response_format": {"type": "json_schema",
                                "json_schema": {"name": "edit", "schema": schema}},
        });
        let endpoint = format!("{}/v1/chat/completions", self.url.trim_end_matches('/'));
        let mut response = ureq::post(&endpoint)
            .send_json(&body)
            .map_err(|e| ForgeError::Backend(e.to_string()))?;
        let json: serde_json::Value = response
            .body_mut()
            .read_json()
            .map_err(|e| ForgeError::Backend(e.to_string()))?;
        let content = json["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| ForgeError::Backend(format!("no content in {json}")))?;
        serde_json::from_str(content).map_err(|e| ForgeError::Backend(format!("bad edit: {e}")))
    }
}

#[derive(Debug)]
pub struct ForgeReport {
    pub iterations: usize,
    pub analyzer_output: String,
}

/// Run `dart analyze`; Ok(None) on clean, Ok(Some(output)) on findings.
pub fn analyze(project: &Path) -> Result<Option<String>, ForgeError> {
    let out = Command::new("dart")
        .arg("analyze")
        .arg(project)
        .output()
        .map_err(|e| ForgeError::Analyzer(format!("running dart analyze: {e}")))?;
    if out.status.success() {
        return Ok(None);
    }
    Ok(Some(format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )))
}

/// The loop: ask the backend for an edit, apply it inside the jail, run
/// the analyzer; feed findings back until clean or out of iterations.
pub fn forge(
    task: &str,
    project: &Path,
    backend: &mut dyn Backend,
    max_iterations: usize,
) -> Result<ForgeReport, ForgeError> {
    let jail = Jail::new(project)?;
    let mut context = String::from("Fresh iteration.");
    for iteration in 1..=max_iterations {
        let edit = backend.edit_for(task, &context)?;
        jail.write(&edit.path, &edit.content)?;
        match analyze(project)? {
            None => {
                return Ok(ForgeReport {
                    iterations: iteration,
                    analyzer_output: String::new(),
                });
            }
            Some(findings) => {
                context = format!(
                    "Your previous edit to `{}` produced analyzer findings:\n{}\n\
                     Reply with a corrected complete file.",
                    edit.path, findings
                );
            }
        }
    }
    Err(ForgeError::NoConvergence(max_iterations))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scripted {
        edits: Vec<Edit>,
    }

    impl Backend for Scripted {
        fn edit_for(&mut self, _task: &str, _context: &str) -> Result<Edit, ForgeError> {
            if self.edits.is_empty() {
                return Err(ForgeError::Backend("script exhausted".into()));
            }
            Ok(self.edits.remove(0))
        }
    }

    fn dart_available() -> bool {
        Command::new("dart").arg("--version").output().is_ok()
    }

    fn dart_project(dir: &Path) {
        std::fs::write(
            dir.join("pubspec.yaml"),
            "name: forge_test\nenvironment:\n  sdk: ^3.0.0\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.join("bin")).unwrap();
    }

    #[test]
    fn loop_converges_when_the_fix_lands() {
        if !dart_available() {
            eprintln!("skipping: dart not on PATH");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        dart_project(dir.path());
        let mut backend = Scripted {
            edits: vec![
                Edit {
                    path: "bin/main.dart".into(),
                    content: "void main() { undefined_symbol(); }\n".into(),
                },
                Edit {
                    path: "bin/main.dart".into(),
                    content: "void main() { print('forged'); }\n".into(),
                },
            ],
        };
        let report = forge("print forged", dir.path(), &mut backend, 3).unwrap();
        assert_eq!(report.iterations, 2, "broken first, fixed second");
    }

    #[test]
    fn loop_reports_no_convergence() {
        if !dart_available() {
            eprintln!("skipping: dart not on PATH");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        dart_project(dir.path());
        let mut backend = Scripted {
            edits: vec![Edit {
                path: "bin/main.dart".into(),
                content: "void main() { broken(; }\n".into(),
            }],
        };
        let err = forge("task", dir.path(), &mut backend, 1);
        assert!(matches!(err, Err(ForgeError::NoConvergence(1))));
    }
}
