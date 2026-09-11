use serde_json::Value;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
use tempfile::TempDir;

fn write(root: &Path, path: &str, contents: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_crap4ts"))
        .arg("--root")
        .arg(root)
        .args(args)
        .output()
        .unwrap()
}

fn json(output: &Output) -> Value {
    assert!(
        output.status.success(),
        "status {:?}\nstdout: {}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn default_paths_and_lcov_produce_correct_asymmetric_scores() {
    let temp = TempDir::new().unwrap();
    write(
        temp.path(),
        "src/functions.ts",
        "function partial(x: boolean) {\n  if (x) {\n    return 1;\n  }\n  return 0;\n}\nfunction uncovered() {\n  return 1;\n}\n",
    );
    write(
        temp.path(),
        "coverage/lcov.info",
        "SF:src/functions.ts\nDA:1,1\nDA:2,0\nDA:7,0\nDA:8,0\nend_of_record\n",
    );

    let rows = json(&run(temp.path(), &["--format", "json"]));
    let rows = rows.as_array().unwrap();
    assert_eq!(rows.len(), 2);

    let partial = rows.iter().find(|row| row["name"] == "partial").unwrap();
    assert_eq!(partial["file"], "src/functions.ts");
    assert_eq!(partial["complexity"], 2);
    assert_eq!(partial["coverage"], 0.5);
    assert_eq!(partial["crap"], 2.5);

    let uncovered = rows.iter().find(|row| row["name"] == "uncovered").unwrap();
    assert_eq!(uncovered["complexity"], 1);
    assert_eq!(uncovered["coverage"], 0.0);
    assert_eq!(uncovered["crap"], 2.0);
}

#[test]
fn sorts_worst_first_and_puts_unknown_coverage_last() {
    let temp = TempDir::new().unwrap();
    write(
        temp.path(),
        "src/a.ts",
        "function low() {}\nfunction unknown() {}\nfunction high(x: boolean) { if (x) return 1; }\n",
    );
    write(
        temp.path(),
        "coverage/lcov.info",
        "SF:src/a.ts\nDA:1,1\nDA:3,0\nend_of_record\n",
    );

    let rows = json(&run(temp.path(), &["--format", "json"]));
    let names: Vec<_> = rows
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["high", "low", "unknown"]);
    assert!(rows[2]["coverage"].is_null());
    assert!(rows[2]["crap"].is_null());
}

#[test]
fn explicit_lcov_path_is_used() {
    let temp = TempDir::new().unwrap();
    write(
        temp.path(),
        "code/app.ts",
        "export function covered() { return 1; }\n",
    );
    write(
        temp.path(),
        "reports/custom.info",
        "SF:code/app.ts\nDA:1,4\nend_of_record\n",
    );

    let rows = json(&run(
        temp.path(),
        &["--format", "json", "--lcov", "reports/custom.info", "code"],
    ));
    assert_eq!(rows[0]["coverage"], 1.0);
    assert_eq!(rows[0]["crap"], 1.0);
}

#[test]
fn threshold_allows_equality_and_exits_two_when_exceeded() {
    let temp = TempDir::new().unwrap();
    write(temp.path(), "src/a.ts", "function f() {}\n");
    write(
        temp.path(),
        "coverage/lcov.info",
        "SF:src/a.ts\nDA:1,0\nend_of_record\n",
    );

    let equal = run(temp.path(), &["--threshold", "2"]);
    assert_eq!(equal.status.code(), Some(0));
    let exceeded = run(temp.path(), &["--threshold", "1.999"]);
    assert_eq!(exceeded.status.code(), Some(2));
}

#[test]
fn require_coverage_fails_when_a_function_has_no_tracked_lines() {
    let temp = TempDir::new().unwrap();
    write(
        temp.path(),
        "src/a.ts",
        "function known() {}\nfunction missing() {}\n",
    );
    write(
        temp.path(),
        "coverage/lcov.info",
        "SF:src/a.ts\nDA:1,1\nend_of_record\n",
    );

    let output = run(temp.path(), &["--require-coverage"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("coverage is missing for one or more functions")
    );
}

#[test]
fn absent_default_coverage_warns_and_emits_nulls() {
    let temp = TempDir::new().unwrap();
    write(temp.path(), "src/a.ts", "function f() {}\n");

    let output = run(temp.path(), &["--format", "json"]);
    let rows = json(&output);
    assert!(String::from_utf8_lossy(&output.stderr).contains("warning: no coverage/lcov.info"));
    assert!(rows[0]["coverage"].is_null());
    assert!(rows[0]["crap"].is_null());
}

#[test]
fn validates_options_and_explicit_missing_coverage() {
    let temp = TempDir::new().unwrap();
    write(temp.path(), "src/a.ts", "function f() {}\n");
    for args in [
        vec!["--threshold", "NaN"],
        vec!["--threshold", "inf"],
        vec!["--threshold=-1"],
        vec!["--unknown"],
        vec!["--lcov", "missing.info"],
    ] {
        assert_eq!(run(temp.path(), &args).status.code(), Some(1), "{args:?}");
    }
    assert_eq!(run(temp.path(), &["--help"]).status.code(), Some(0));
}

#[test]
fn text_report_shows_percentages_and_unknowns() {
    let temp = TempDir::new().unwrap();
    write(
        temp.path(),
        "src/a.ts",
        "function f(x: boolean) {\n if (x) return 1;\n}\nfunction missing() {}\n",
    );
    write(
        temp.path(),
        "coverage/lcov.info",
        "SF:src/a.ts\nDA:1,1\nDA:2,0\nend_of_record\n",
    );
    let output = run(temp.path(), &[]);
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<_> = text.lines().collect();
    assert_eq!(lines[0], "CRAP Report");
    assert_eq!(
        lines[3].split_whitespace().collect::<Vec<_>>(),
        ["f", "src/a.ts:1", "2", "50.0", "2.5"]
    );
    assert_eq!(
        lines[4].split_whitespace().collect::<Vec<_>>(),
        ["missing", "src/a.ts:4", "1", "N/A", "N/A"]
    );
}

#[test]
fn malformed_sources_lcov_and_nonexistent_inputs_are_errors() {
    let temp = TempDir::new().unwrap();
    write(temp.path(), "bad.ts", "const value = ;\n");
    write(
        temp.path(),
        "bad.info",
        "SF:bad.ts\nDA:zero,1\nend_of_record\n",
    );
    write(temp.path(), "good.ts", "function good() {}\n");

    let bad_ts = run(temp.path(), &["bad.ts"]);
    assert_eq!(bad_ts.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&bad_ts.stderr).contains("syntax error"));

    let bad_lcov = run(temp.path(), &["--lcov", "bad.info", "good.ts"]);
    assert_eq!(bad_lcov.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&bad_lcov.stderr).contains("invalid DA line number"));

    let missing = run(temp.path(), &["does-not-exist.ts"]);
    assert_eq!(missing.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&missing.stderr).contains("source path does not exist"));
}

#[test]
fn ignores_generated_directories_and_declarations_deduplicates_and_parses_tsx() {
    let temp = TempDir::new().unwrap();
    write(
        temp.path(),
        "project/view.tsx",
        "export const View = () => <div>{ok ? <b /> : null}</div>;\n",
    );
    write(
        temp.path(),
        "project/types.d.ts",
        "export declare function declarationOnly(): void;\n",
    );
    write(
        temp.path(),
        "project/node_modules/pkg/a.ts",
        "function dependency() {}\n",
    );
    write(
        temp.path(),
        "project/dist/a.ts",
        "function generated() {}\n",
    );

    let rows = json(&run(
        temp.path(),
        &["--format", "json", "project", "project/view.tsx"],
    ));
    let rows = rows.as_array().unwrap();
    assert_eq!(rows.len(), 1, "rows: {rows:?}");
    assert_eq!(rows[0]["name"], "View");
    assert_eq!(rows[0]["complexity"], 2);
    assert_eq!(rows[0]["file"], "project/view.tsx");
}
