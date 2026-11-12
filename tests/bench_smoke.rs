//! Smoke run of the benchmark on a 2 s corpus: times are positive and the
//! table parses. No speed thresholds. The reference encoders are skipped so
//! CI needs neither LAME nor ffmpeg.

#[path = "../examples/bench.rs"]
#[allow(dead_code)]
mod bench;

fn args(extra: &[&str]) -> Vec<String> {
    ["--seconds", "2", "--runs", "1", "--no-reference"]
        .iter()
        .chain(extra)
        .map(|s| s.to_string())
        .collect()
}

#[test]
fn table_parses_and_times_are_positive() {
    let out = bench::run(&args(&[])).unwrap();
    let rows: Vec<Vec<&str>> = out
        .lines()
        .filter(|l| l.starts_with('|'))
        .map(|l| l.trim_matches('|').split('|').map(str::trim).collect())
        .collect();
    assert!(rows.len() > 1 + 28);
    let width = rows[0].len();
    assert_eq!(width, 10);
    let mut craft = 0;
    for r in &rows[2..] {
        assert_eq!(r.len(), width, "{r:?}");
        if r[2] == "craft-mp3" {
            craft += 1;
            assert!(r[3].parse::<f64>().unwrap() > 0.0, "{r:?}");
            assert!(r[5].parse::<f64>().unwrap() > 0.0, "{r:?}");
        } else {
            assert_eq!(r[3], "not found");
        }
    }
    assert_eq!(craft, 28);
}

#[test]
fn json_has_a_row_per_case() {
    let out = bench::run(&args(&["--json"])).unwrap();
    assert!(out.starts_with("{\"machine\":"));
    assert_eq!(out.matches("\"encoder\":\"craft-mp3\"").count(), 28);
    assert_eq!(out.matches('{').count(), out.matches('}').count());
}
