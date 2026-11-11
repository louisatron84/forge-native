//! Port of shared/dataSummary.test.ts.
use forge_core::data_summary::*;
use serde_json::{json, Value};
fn summary(value: Value) -> Summary {
    serde_json::from_value(value).unwrap()
}
fn df(rows: u64, cols: &[&str]) -> Summary {
    summary(
        json!({"type":"pandas.DataFrame","kind":"dataframe","shape":[rows,cols.len()],"columns":cols.iter().map(|c|[*c,"int64"]).collect::<Vec<_>>(),"rows":[]}),
    )
}
#[test]
fn describes_all_value_kinds() {
    assert_eq!(summary_text(&df(1204, &["a", "b", "c"])), "table: 1,204 rows × 3 columns");
    for (s, expected) in [
        (json!({"type":"pandas.Series","kind":"series","shape":[10],"dtype":"float64"}), "column: 10 values (float64)"),
        (json!({"type":"list","kind":"sequence","len":3}), "list of 3"),
        (json!({"type":"dict","kind":"mapping","len":1}), "dict of 1"),
        (json!({"type":"int","kind":"value","repr":"42"}), "42"),
    ] {
        assert_eq!(summary_text(&summary(s)), expected);
    }
}
#[test]
fn describes_filtering_grouping_and_column_changes() {
    assert_eq!(change(Some(&df(1204, &["a", "b"])), Some(&df(873, &["a", "b"])), "filter"), "1,204 → 873 rows (−331)");
    assert_eq!(
        change(Some(&df(10, &["a", "b"])), Some(&df(10, &["a", "b", "total"])), "transform"),
        "+1 column: total"
    );
    assert_eq!(change(Some(&df(10, &["a", "b", "c"])), Some(&df(10, &["a"])), "select"), "−2 columns: b, c");
    assert_eq!(
        change(Some(&df(100, &["region", "amount"])), Some(&df(4, &["region", "amount"])), "group"),
        "100 rows → 4 groups"
    );
    assert_eq!(change(Some(&df(5, &["a"])), Some(&df(5, &["a"])), "transform"), "");
    assert_eq!(change(None, Some(&df(5, &["a"])), "read"), "");
}
#[test]
fn parses_data_and_io_and_rejects_malformed_messages() {
    let m =
        json!({"t":"data","func":"m:f","line":3,"var":"x","n":1,"summary":{"type":"int","kind":"value","repr":"1"}});
    assert!(matches!(parse_data_message(&m), Some(DataMessage::Data { line: 3, n: 1, .. })));
    let m = json!({"t":"io","io":{"func":"m:g","total":3,"calls":[{"in":{"a":{"type":"int","kind":"value","repr":"1"},"bad":null},"out":null},null]}});
    let Some(DataMessage::Io(io)) = parse_data_message(&m) else { panic!("io missing") };
    assert_eq!(io.total, 3);
    assert_eq!(io.calls.len(), 1);
    assert_eq!(io.calls[0].input.len(), 1);
    for m in [
        json!(null),
        json!({"t":"tick"}),
        json!({"t":"data","func":"m:f"}),
        json!({"t":"data","func":"m:f","line":1.5,"var":"x","summary":{}}),
    ] {
        assert!(parse_data_message(&m).is_none());
    }
}
#[test]
fn keeps_latest_value_per_line_and_resets_between_runs() {
    let mut log = DataLog::default();
    log.begin(1);
    for n in 1..=2 {
        log.add(parse_data_message(&json!({"t":"data","func":"m:f","line":3,"var":"x","n":n,"summary":{"type":"int","kind":"value","repr":n.to_string()}})).unwrap());
    }
    log.add(parse_data_message(&json!({"t":"io","io":{"func":"m:g","total":1,"calls":[]}})).unwrap());
    let points = log.between("m:f", 3, 3);
    assert_eq!(points.len(), 1);
    assert_eq!(points[0].n, 2);
    assert_eq!(points[0].summary.repr.as_deref(), Some("2"));
    assert_eq!(log.io("m:g").unwrap().total, 1);
    assert_eq!(log.func.as_deref(), Some("m:f"));
    assert_eq!(log.between("m:f", 0, u32::MAX).len(), 1);
    log.begin(2);
    assert!(!log.has_data());
    assert!(log.between("m:f", 3, 3).is_empty());
}

#[test]
fn capture_limit_keeps_existing_points_updatable_and_reports_drops() {
    let mut log = DataLog::default();
    for line in 1..=20_002 {
        log.add(parse_data_message(&json!({"t":"data","func":"m:f","line":line,"var":"x","summary":{"type":"int","kind":"value","repr":"1"}})).unwrap());
    }
    assert_eq!(log.between("m:f", 0, u32::MAX).len(), 20_000);
    assert_eq!(log.dropped, 2);
    log.add(parse_data_message(&json!({"t":"data","func":"m:f","line":1,"var":"x","n":4,"summary":{"type":"int","kind":"value","repr":"9"}})).unwrap());
    assert_eq!(log.between("m:f", 1, 1)[0].n, 4);
}
