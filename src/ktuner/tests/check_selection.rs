use std::collections::BTreeSet;
use std::process::{Command, Output};

fn ktuner(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ktuner"))
        .args(arguments)
        .output()
        .expect("run ktuner")
}

fn check(arguments: &[&str]) -> Output {
    let mut args = vec!["check"];
    args.extend_from_slice(arguments);
    let output = ktuner(&args);
    assert!(
        matches!(output.status.code(), Some(0 | 1)),
        "ktuner {args:?} must exit 0 or 1: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn body(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "ktuner check must print JSON on stdout: {error}; stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn params(body: &serde_json::Value) -> BTreeSet<String> {
    body["recommendations"]
        .as_array()
        .expect("check must print recommendations")
        .iter()
        .map(|entry| {
            entry["param"]
                .as_str()
                .expect("recommendation must name its parameter")
                .to_string()
        })
        .collect()
}

fn confidence_weight(entry: &serde_json::Value) -> usize {
    match entry["confidence"].as_str() {
        Some("high") => 3,
        Some("medium") => 2,
        other => panic!("unexpected confidence {other:?} in {entry}"),
    }
}

fn category_filter(entry: &serde_json::Value) -> Option<&'static str> {
    if entry["category"] == "security" {
        return Some("security");
    }
    match entry["subcategory"].as_str()? {
        "network" => Some("net"),
        "memory" => Some("mem"),
        "io" => Some("io"),
        "cpu" => Some("cpu"),
        _ => None,
    }
}

fn slash_alias(param: &str) -> Option<String> {
    for prefix in [
        "net.ipv4.conf.",
        "net.ipv6.conf.",
        "net.ipv4.neigh.",
        "net.ipv6.neigh.",
    ] {
        if let Some(rest) = param.strip_prefix(prefix) {
            let (interface, property) = rest.rsplit_once('.')?;
            if interface.contains('.') {
                return None;
            }
            return Some(format!("{prefix}{interface}.{property}").replace('.', "/"));
        }
    }
    Some(param.replace('.', "/"))
}

#[test]
fn check_only_and_exclude_filter_the_report_without_changing_system_totals() {
    let baseline_output = check(&[]);
    let baseline = body(&baseline_output);
    assert!(
        baseline.get("unmatched_only").is_none() && baseline.get("unmatched_exclude").is_none(),
        "an unfiltered check must keep its existing JSON shape: {baseline}"
    );
    let findings = baseline["recommendations"]
        .as_array()
        .expect("check must print recommendations");
    if findings.len() < 2 {
        eprintln!("skip: this host has fewer than two recommendations");
        return;
    }

    let first = findings[0]["param"].as_str().expect("param");
    let Some(second) = findings
        .iter()
        .find(|entry| entry["param"].as_str() != Some(first))
        .and_then(|entry| entry["param"].as_str())
    else {
        eprintln!("skip: this host has fewer than two distinct recommendations");
        return;
    };
    let only_output = check(&["--only", first, "--only", second]);
    let only = body(&only_output);
    assert_eq!(
        params(&only),
        BTreeSet::from([first.to_string(), second.to_string()])
    );
    assert!(only.get("unmatched_only").is_none());
    assert_eq!(only["score"], baseline["score"]);
    assert_eq!(only["total_checked"], baseline["total_checked"]);
    assert_eq!(only_output.status.code(), Some(1));
    let selected = only["recommendations"].as_array().expect("recommendations");
    let applied: BTreeSet<String> = selected
        .iter()
        .filter(|entry| {
            entry["writable"].as_bool() == Some(true) && entry["param"] != "vm.nr_hugepages"
        })
        .map(|entry| entry["param"].as_str().expect("param").to_string())
        .collect();
    let remaining_penalty: usize = findings
        .iter()
        .filter(|entry| {
            !entry["param"]
                .as_str()
                .is_some_and(|param| applied.contains(param))
        })
        .map(confidence_weight)
        .sum();
    assert_eq!(
        only["predicted_score"],
        100usize.saturating_sub(remaining_penalty).max(30)
    );
    assert_eq!(
        only["counts"]["performance"],
        selected
            .iter()
            .filter(|entry| entry["category"] == "performance")
            .count()
    );
    assert_eq!(
        only["counts"]["security"],
        selected
            .iter()
            .filter(|entry| entry["category"] == "security")
            .count()
    );
    assert_eq!(
        only["counts"]["high_confidence"],
        selected
            .iter()
            .filter(|entry| entry["confidence"] == "high")
            .count()
    );
    assert_eq!(
        only["counts"]["writable"],
        selected
            .iter()
            .filter(|entry| entry["writable"].as_bool() == Some(true))
            .count()
    );

    let exclude_output = check(&["--exclude", first]);
    let exclude = body(&exclude_output);
    let mut expected = params(&baseline);
    expected.remove(first);
    assert_eq!(params(&exclude), expected);
    assert_eq!(exclude["score"], baseline["score"]);
    assert_eq!(exclude["total_checked"], baseline["total_checked"]);
    assert_eq!(exclude_output.status.code(), baseline_output.status.code());

    // Slash and dot forms name the same sysctl parameter in every ktuner
    // command. Literal dots inside interface identities are preserved.
    if let Some(alias) = slash_alias(first) {
        let alias_output = check(&["--only", &alias]);
        assert_eq!(
            params(&body(&alias_output)),
            BTreeSet::from([first.to_string()])
        );
    }
}

#[test]
fn check_selection_applies_after_category_and_conservative_filters() {
    let baseline = body(&check(&[]));
    let findings = baseline["recommendations"]
        .as_array()
        .expect("check must print recommendations");
    let Some(first) = findings.first() else {
        eprintln!("skip: this host has no recommendations");
        return;
    };
    if let Some(category) = category_filter(first) {
        let first_param = first["param"].as_str().expect("param");
        let scoped = check(&["--category", category, "--only", first_param]);
        assert_eq!(
            params(&body(&scoped)),
            BTreeSet::from([first_param.to_string()])
        );

        if let Some(outside_category) = findings
            .iter()
            .find(|entry| category_filter(entry) != Some(category))
        {
            let outside_param = outside_category["param"].as_str().expect("param");
            let filtered = check(&["--category", category, "--only", outside_param]);
            let filtered_body = body(&filtered);
            assert!(params(&filtered_body).is_empty());
            assert_eq!(
                filtered_body["unmatched_only"],
                serde_json::json!([outside_param])
            );
        }
    }

    if let Some(medium) = findings.iter().find(|entry| entry["confidence"] != "high") {
        let param = medium["param"].as_str().expect("param");
        let filtered = check(&["--conservative", "--only", param]);
        let filtered_body = body(&filtered);
        assert!(params(&filtered_body).is_empty());
        assert_eq!(filtered_body["unmatched_only"], serde_json::json!([param]));
    }
}

#[test]
fn check_reports_names_that_match_nothing_in_the_filtered_scope() {
    let missing = "no_such_ktuner_parameter";
    let only_output = check(&["--only", missing]);
    let only = body(&only_output);
    assert!(params(&only).is_empty());
    assert_eq!(only["unmatched_only"], serde_json::json!([missing]));
    assert_eq!(only_output.status.code(), Some(0));

    let exclude_output = check(&["--exclude", missing]);
    let exclude = body(&exclude_output);
    assert_eq!(params(&exclude), params(&body(&check(&[]))));
    assert_eq!(exclude["unmatched_exclude"], serde_json::json!([missing]));
}

#[test]
fn check_rejects_mutually_exclusive_selections_as_json_usage_error() {
    let output = ktuner(&[
        "check",
        "--only",
        "vm.swappiness",
        "--exclude",
        "fs.file-max",
    ]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let error: serde_json::Value =
        serde_json::from_slice(&output.stderr).expect("usage errors are JSON on stderr");
    assert!(error["error"]
        .as_str()
        .is_some_and(|message| !message.is_empty()));
}
