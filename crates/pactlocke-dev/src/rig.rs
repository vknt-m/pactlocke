//! `cargo dev rig-check <dir> <player>...`: reads each Player's report from
//! a test rig run (`<dir>/<player>/report.json`, written by the headless
//! app) and checks them against each other: every script passed, every
//! Shadow Game matched, and everyone has the same Run log.

use serde_json::Value;

/// Checks a rig run's reports, by Player in join order. Returns a summary
/// line per Player, and every problem found.
pub fn check(reports: &[(String, Option<Value>)]) -> (Vec<String>, Vec<String>) {
    let mut summary = Vec::new();
    let mut problems = Vec::new();
    // The Run log of each Player who stayed to the end: (who, entries, hash).
    let mut logs: Vec<(&str, &[Value], Option<&str>)> = Vec::new();
    for (name, report) in reports {
        let Some(report) = report else {
            problems.push(format!("{name}: no report (the app didn't get to write one)"));
            continue;
        };
        let vanished = report["vanished"].as_bool() == Some(true);
        let entries = report["log"]["entries"].as_array().map_or(&[][..], Vec::as_slice);
        let shadows = report["shadows"].as_array().map_or(&[][..], Vec::as_slice);
        let frames = report["frames"].as_u64().unwrap_or(0);
        summary.push(format!(
            "{name}: {} steps passed, {frames} frames, {} log entries, Shadow Games: {}{}",
            report["passed"].as_array().map_or(0, Vec::len),
            entries.len(),
            shadows
                .iter()
                .map(|s| format!(
                    "{} {}",
                    s["name"].as_str().unwrap_or("?"),
                    s["state"].as_str().unwrap_or("?")
                ))
                .collect::<Vec<_>>()
                .join(", "),
            if vanished { " (vanished, as scripted)" } else { "" },
        ));
        if let Some(failed) = report["failed"].as_object() {
            problems.push(format!(
                "{name}: line {} failed: {}",
                failed.get("line").map_or_else(|| "?".into(), ToString::to_string),
                failed.get("why").and_then(Value::as_str).unwrap_or("?")
            ));
        }
        if vanished {
            continue;
        }
        for shadow in shadows {
            if shadow["state"].as_str() != Some("Matched") {
                problems.push(format!(
                    "{name}: the Shadow Game of {} ended {}",
                    shadow["name"].as_str().unwrap_or("?"),
                    shadow["state"].as_str().unwrap_or("?")
                ));
            }
        }
        logs.push((name, entries, report["log"]["hash"].as_str()));
    }
    problems.extend(logs_agree(&logs));
    (summary, problems)
}

/// Every Player's Run log is the same, or one a little behind another's: a
/// shorter log must be the start of the longest, entry for entry.
fn logs_agree(logs: &[(&str, &[Value], Option<&str>)]) -> Vec<String> {
    let Some((longest, most, longest_hash)) = logs.iter().max_by_key(|(_, entries, _)| entries.len()) else {
        return Vec::new();
    };
    let mut problems = Vec::new();
    for (name, entries, hash) in logs {
        if let Some(i) = entries.iter().zip(most.iter()).position(|(a, b)| a != b) {
            problems.push(format!(
                "{name}'s Run log differs from {longest}'s at entry {i}: {} against {}",
                entries[i], most[i]
            ));
        } else if entries.len() == most.len() && hash != longest_hash {
            problems.push(format!(
                "{name}'s Run log has the same entries as {longest}'s but another hash"
            ));
        } else if entries.len() + 2 < most.len() {
            problems.push(format!(
                "{name}'s Run log is {} entries behind {longest}'s",
                most.len() - entries.len()
            ));
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn report(entries: &[&str], hash: &str, shadow: &str) -> Option<Value> {
        let entries: Vec<Value> = entries
            .iter()
            .enumerate()
            .map(|(i, kind)| json!({ "seq": i, "by": "Red", "kind": kind }))
            .collect();
        Some(json!({
            "passed": [1, 2, 3],
            "failed": null,
            "vanished": false,
            "frames": 100,
            "log": { "entries": entries, "hash": hash },
            "shadows": [{ "name": "Other", "state": shadow }],
        }))
    }

    #[test]
    fn matching_reports_pass() {
        let reports = vec![
            ("Red".to_owned(), report(&["genesis", "locked"], "aa", "Matched")),
            ("Blue".to_owned(), report(&["genesis", "locked"], "aa", "Matched")),
        ];
        let (summary, problems) = check(&reports);
        assert!(problems.is_empty(), "{problems:?}");
        assert!(summary[0].starts_with("Red: 3 steps passed, 100 frames, 2 log entries, Shadow Games: Other Matched"));
    }

    #[test]
    fn a_failed_step_a_diverged_shadow_and_a_different_log_are_each_a_problem() {
        let mut failed = report(&["genesis", "locked"], "aa", "Matched").unwrap();
        failed["failed"] = json!({ "line": 7, "why": "not met in 3000 frames" });
        let reports = vec![
            ("Red".to_owned(), Some(failed)),
            (
                "Blue".to_owned(),
                report(&["genesis", "retry"], "bb", "Diverged { frame: 9 }"),
            ),
            ("Green".to_owned(), None),
        ];
        let (_, problems) = check(&reports);
        assert_eq!(problems.len(), 4, "{problems:?}");
        assert!(problems[0].contains("line 7 failed: not met"));
        assert!(problems[1].contains("Shadow Game of Other ended Diverged"));
        assert!(problems[2].contains("Green: no report"));
        assert!(problems[3].contains("differs") && problems[3].contains("entry 1"));
    }

    #[test]
    fn a_log_a_little_behind_is_fine_and_a_vanished_player_is_left_out() {
        let mut gone = report(&["genesis"], "cc", "Diverged { frame: 1 }").unwrap();
        gone["vanished"] = json!(true);
        let reports = vec![
            ("Red".to_owned(), Some(gone)),
            (
                "Blue".to_owned(),
                report(&["genesis", "locked", "event"], "aa", "Matched"),
            ),
            ("Green".to_owned(), report(&["genesis", "locked"], "bb", "Matched")),
        ];
        let (summary, problems) = check(&reports);
        assert!(problems.is_empty(), "{problems:?}");
        assert!(summary[0].ends_with("(vanished, as scripted)"));
    }
}
