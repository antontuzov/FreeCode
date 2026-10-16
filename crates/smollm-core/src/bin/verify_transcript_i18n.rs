//! CI gate: the Rust transcript vocabulary and the UI's i18n catalogs must not drift apart.
//!
//! `StepKind::i18n_key` is the contract — the Rust side names a row, the catalog owns its wording.
//! A key missing from `en.json` renders as raw `transcript.step.ranTools` inside the transcript, and
//! a key missing from `zh.json` silently falls back to English in the middle of the Chinese task
//! summary the reference screenshot shows. Both are invisible until the app is running, so this
//! binary fails the build instead.
//!
//! Notices run on the same contract, one step further: the host stopped writing the sentence at all
//! and sends only `NoticeCode` plus the fields it measured, so the catalog is the *only* copy. Each
//! code's entry is then required in every locale, and its `{{braces}}` have to be exactly the fields
//! `NoticeCode::params` names — otherwise a notice renders its key, or a number that went out under a
//! different name leaves a hole in the middle of the line.
//!
//! Usage: `cargo run -p smollm-core --bin verify-transcript-i18n [i18n-dir]`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use smollm_core::{all_i18n_keys, NoticeCode};

const LOCALES: [&str; 2] = ["en", "zh"];
const STEP_PREFIX: &str = "transcript.step.";
const NOTICE_PREFIX: &str = "notice.";

fn main() {
    let dir = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(default_dir);
    let failures = check(&dir);
    if failures.is_empty() {
        println!(
            "verify-transcript-i18n: OK — {} step keys and {} notice keys present in {:?}",
            all_i18n_keys().len(),
            NoticeCode::ALL.len(),
            LOCALES
        );
        return;
    }
    for failure in &failures {
        eprintln!("  FAIL {failure}");
    }
    eprintln!(
        "verify-transcript-i18n: {} failure(s), looking in {}",
        failures.len(),
        dir.display()
    );
    std::process::exit(1);
}

fn default_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/desktop/src/i18n")
}

fn check(dir: &Path) -> Vec<String> {
    let mut failures = Vec::new();
    let catalogs: Vec<(String, BTreeMap<String, String>)> = LOCALES
        .iter()
        .filter_map(|locale| match load(dir, locale) {
            Ok(catalog) => Some(((*locale).to_string(), catalog)),
            Err(e) => {
                failures.push(e);
                None
            }
        })
        .collect();

    for key in all_i18n_keys() {
        for (locale, catalog) in &catalogs {
            match catalog.get(key) {
                None => failures.push(format!("{locale}.json has no {key}")),
                Some(text) if text.trim().is_empty() => {
                    failures.push(format!("{locale}.json {key} is blank"));
                }
                Some(_) => {}
            }
        }
    }

    // A notice's sentence belongs to the catalog alone, so this is the only place the two halves of
    // that contract meet: each code the host can emit needs one entry per locale, interpolating
    // exactly the fields the host measures. A brace the host never fills reads as a blank in the
    // middle of the sentence, and a field the host sends that no locale names is a number nobody
    // reads — neither fails anything until the notice is on screen in the other language.
    for code in NoticeCode::ALL {
        let key = code.i18n_key();
        let sent: BTreeSet<String> = code
            .params()
            .iter()
            .map(|name| (*name).to_string())
            .collect();
        for (locale, catalog) in &catalogs {
            match catalog.get(key) {
                None => failures.push(format!("{locale}.json has no {key}")),
                Some(text) => {
                    let got = placeholders(text);
                    if got != sent {
                        failures.push(format!(
                            "{locale}.json {key} interpolates {got:?}, the host sends {sent:?}"
                        ));
                    }
                }
            }
        }
    }

    // Copy no code emits is a sentence that can never be read.
    let codes: BTreeSet<String> = NoticeCode::ALL
        .iter()
        .map(|code| code.i18n_key().to_string())
        .collect();
    for (locale, catalog) in &catalogs {
        for key in catalog.keys().filter(|k| k.starts_with(NOTICE_PREFIX)) {
            if !codes.contains(key) {
                failures.push(format!(
                    "{locale}.json defines {key}, which no NoticeCode emits"
                ));
            }
        }
    }

    // A step row the renderer knows but the run model does not is a row that can never appear.
    let known: BTreeSet<&str> = all_i18n_keys().into_iter().collect();
    for (locale, catalog) in &catalogs {
        for key in catalog.keys().filter(|k| k.starts_with(STEP_PREFIX)) {
            if !known.contains(key.as_str()) {
                failures.push(format!(
                    "{locale}.json defines {key}, which no StepKind::i18n_key produces"
                ));
            }
        }
    }

    if catalogs.len() == LOCALES.len() {
        let (base_name, base) = &catalogs[0];
        for (locale, catalog) in &catalogs[1..] {
            for key in base.keys().filter(|k| !catalog.contains_key(*k)) {
                failures.push(format!(
                    "{locale}.json is missing {key}, which {base_name}.json has"
                ));
            }
            for key in catalog.keys().filter(|k| !base.contains_key(*k)) {
                failures.push(format!(
                    "{locale}.json adds {key}, which {base_name}.json lacks"
                ));
            }
            for (key, text) in catalog {
                if let Some(expected) = base.get(key) {
                    let (want, got) = (placeholders(expected), placeholders(text));
                    if want != got {
                        failures.push(format!(
                            "{locale}.json {key} interpolates {got:?}, {base_name}.json uses {want:?}"
                        ));
                    }
                }
            }
        }
    }

    failures
}

/// Read one catalog. Flat dotted keys are the shape the app ships, but a nested object flattens the
/// same way so a restructure cannot make this check vacuously pass.
fn load(dir: &Path, locale: &str) -> Result<BTreeMap<String, String>, String> {
    let path = dir.join(format!("{locale}.json"));
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let value: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| format!("cannot parse {}: {e}", path.display()))?;
    let mut out = BTreeMap::new();
    flatten("", &value, &mut out);
    if out.is_empty() {
        return Err(format!("{} holds no string keys", path.display()));
    }
    Ok(out)
}

fn flatten(prefix: &str, node: &serde_json::Value, out: &mut BTreeMap<String, String>) {
    match node {
        serde_json::Value::Object(map) => {
            for (key, child) in map {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                flatten(&path, child, out);
            }
        }
        serde_json::Value::String(text) => {
            out.insert(prefix.to_string(), text.clone());
        }
        _ => {}
    }
}

fn placeholders(text: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        match after.find("}}") {
            Some(end) => {
                names.insert(after[..end].trim().to_string());
                rest = &after[end + 2..];
            }
            None => break,
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(writes: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (name, body) in writes {
            std::fs::write(dir.path().join(name), *body).unwrap();
        }
        dir
    }

    /// A catalog that satisfies both vocabularies: every step row, and every notice's sentence with
    /// exactly the fields its code sends. Tests that probe one drift start from this.
    fn complete() -> BTreeMap<String, String> {
        let mut map: BTreeMap<String, String> = BTreeMap::new();
        for key in all_i18n_keys() {
            map.insert(key.to_string(), "x".into());
        }
        for code in NoticeCode::ALL {
            let body = code
                .params()
                .iter()
                .map(|name| format!("{{{{{name}}}}}"))
                .collect::<Vec<_>>()
                .join(" ");
            map.insert(code.i18n_key().to_string(), body);
        }
        map
    }

    fn write_both(dir: &Path, map: &BTreeMap<String, String>) {
        let body = serde_json::to_string(map).unwrap();
        std::fs::write(dir.join("en.json"), &body).unwrap();
        std::fs::write(dir.join("zh.json"), &body).unwrap();
    }

    #[test]
    fn the_repo_catalogs_pass_the_gate() {
        let failures = check(&default_dir());
        assert!(failures.is_empty(), "{failures:?}");
    }

    #[test]
    fn a_notice_with_no_sentence_is_named() {
        let dir = tempfile::tempdir().unwrap();
        let mut map = complete();
        map.remove("notice.budget-exhausted").unwrap();
        write_both(dir.path(), &map);
        let failures = check(dir.path());
        assert_eq!(failures.len(), 2, "{failures:?}");
        assert!(
            failures
                .iter()
                .all(|f| f.contains("notice.budget-exhausted")),
            "{failures:?}"
        );
    }

    #[test]
    fn a_notice_whose_sentence_drops_a_measured_field_is_named() {
        let dir = tempfile::tempdir().unwrap();
        // The host still sends the window width; the copy stopped using it, and the notice would read
        // "…and the window is  wide" on screen.
        let mut map = complete();
        map.insert(
            "notice.prompt-too-long".to_string(),
            "too big: {{tokens}} tokens".into(),
        );
        write_both(dir.path(), &map);
        let failures = check(dir.path());
        assert!(
            failures
                .iter()
                .any(|f| f.contains("notice.prompt-too-long") && f.contains("window")),
            "{failures:?}"
        );
    }

    #[test]
    fn a_notice_sentence_no_code_emits_is_rejected() {
        let mut map = complete();
        map.insert(
            format!("{NOTICE_PREFIX}gpu-overheated"),
            "{{degrees}}".into(),
        );
        let dir = tempfile::tempdir().unwrap();
        write_both(dir.path(), &map);
        let failures = check(dir.path());
        assert_eq!(failures.len(), 2, "{failures:?}");
        assert!(failures.iter().all(|f| f.contains("gpu-overheated")));
    }

    #[test]
    fn a_dropped_step_key_is_named() {
        let dir = tempfile::tempdir().unwrap();
        let body = r#"{"transcript.step.processed":"Processed","app.name":"SmoLLM Studio"}"#;
        std::fs::write(dir.path().join("en.json"), body).unwrap();
        std::fs::write(dir.path().join("zh.json"), body).unwrap();
        let failures = check(dir.path());
        assert!(
            failures
                .iter()
                .any(|f| f.contains("en.json has no transcript.step.ranTools")),
            "{failures:?}"
        );
    }

    #[test]
    fn a_step_key_with_no_rust_meaning_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        // Both vocabularies are otherwise complete, so the only thing left to flag is the stray.
        let mut map = complete();
        map.insert(format!("{STEP_PREFIX}polishText"), "x".into());
        write_both(dir.path(), &map);
        let failures = check(dir.path());
        assert_eq!(failures.len(), 2, "{failures:?}");
        assert!(failures.iter().all(|f| f.contains("polishText")));
    }

    #[test]
    fn interpolation_arguments_must_match_across_locales() {
        let dir = fixture(&[
            ("en.json", "{\"a\":\"Ran {{count}} tools\"}"),
            ("zh.json", "{\"a\":\"运行了 个工具\"}"),
        ]);
        let failures = check(dir.path());
        assert!(
            failures.iter().any(|f| f.contains("interpolates")),
            "{failures:?}"
        );
    }

    #[test]
    fn nested_json_flattens_instead_of_passing_vacuously() {
        let dir = fixture(&[
            (
                "en.json",
                "{\"transcript\":{\"step\":{\"processed\":\"Processed\"}}}",
            ),
            (
                "zh.json",
                "{\"transcript\":{\"step\":{\"processed\":\"已处理\"}}}",
            ),
        ]);
        let catalog = load(dir.path(), "en").unwrap();
        assert_eq!(
            catalog.get("transcript.step.processed").unwrap(),
            "Processed"
        );
    }
}
