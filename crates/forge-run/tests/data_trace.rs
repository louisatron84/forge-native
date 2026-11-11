//! Real-Python data/io cases from main/runTrace.test.ts, plus real pandas.
use forge_core::data_summary::{change, parse_data_message, DataLog, DataMessage};
use std::{collections::BTreeMap, time::Duration};
fn run(source: &str) -> Option<Vec<DataMessage>> {
    let dir = tempfile::tempdir().unwrap();
    let python = forge_run::python::find_python(dir.path(), None)?;
    let path = dir.path().join("data.py");
    std::fs::write(&path, source).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let server = forge_run::trace::TraceServer::open(move |m| {
        let _ = tx.send(m);
    })
    .unwrap();
    let mut env = server.env(dir.path());
    env.insert("FORGE_TRACE_DATA".into(), "data:pipeline".into());
    let env = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect::<Vec<_>>();
    let result = forge_sys::process::run_captured(
        &python,
        &forge_run::trace::trace_command(&[path.to_string_lossy().into()], &[]),
        dir.path(),
        &env,
        "",
        Duration::from_secs(30),
    );
    assert!(result.ok(), "{result:?}");
    assert!(result.stderr.is_empty(), "{}", result.stderr);
    let mut messages = Vec::new();
    loop {
        let m = rx.recv_timeout(Duration::from_secs(4)).expect("Tracer end");
        let end = m["t"] == "end";
        if let Some(m) = parse_data_message(&m) {
            messages.push(m);
        }
        if end {
            break;
        }
    }
    Some(messages)
}
const SOURCE: &str = r#"class Dtypes:
    def __init__(self, pairs):
        self.pairs = pairs
    def items(self):
        return iter(self.pairs)

class Frame:
    """Looks like a pandas DataFrame (pandas may not be installed)."""
    def __init__(self, rows, cols):
        self.rows, self.cols = rows, cols
    shape = property(lambda self: (len(self.rows), len(self.cols)))
    columns = property(lambda self: self.cols)
    dtypes = property(lambda self: Dtypes([(c, "int64") for c in self.cols]))
    index = property(lambda self: list(range(len(self.rows))))
    def head(self, n):
        return Frame(self.rows[:n], self.cols)
    def itertuples(self, index=False, name=None):
        return iter([tuple(r) for r in self.rows])

def load(n):
    return Frame([[i, i * i] for i in range(n)], ["a", "b"])

def pipeline(n):
    df = load(n)
    small = df.head(2)
    names = [{"x": 1, "y": "a"}, {"x": 2}]
    total = 0
    for i in range(1, 6):
        total += i
    return small

pipeline(10)
pipeline(3)
"#;
#[test]
fn watched_function_reports_tables_loops_first_call_and_function_io() {
    let Some(messages) = run(SOURCE) else {
        return;
    };
    let mut points: BTreeMap<String, Vec<_>> = BTreeMap::new();
    let mut log = DataLog::default();
    for m in messages {
        if let DataMessage::Data { ref func, line, ref var, ref summary, .. } = m {
            assert_eq!(func, "data:pipeline");
            points.entry(var.clone()).or_default().push((line, summary.clone()));
        }
        log.add(m);
    }
    let df = &points["df"];
    assert_eq!(df.len(), 1);
    assert_eq!(df[0].0, 24);
    assert_eq!(df[0].1.shape.as_deref(), Some([10, 2].as_slice()));
    assert_eq!(
        df[0].1.rows.as_ref().unwrap(),
        &vec![vec!["0", "0"], vec!["1", "1"], vec!["2", "4"], vec!["3", "9"], vec!["4", "16"]]
    );
    assert_eq!(points["small"][0].1.shape.as_deref(), Some([2, 2].as_slice()));
    assert_eq!(points["names"][0].1.rows.as_ref().unwrap(), &vec![vec!["1", "'a'"], vec!["2", "None"]]);
    assert_eq!(
        points["total"].iter().map(|(_, s)| s.repr.as_deref().unwrap()).collect::<Vec<_>>(),
        ["0", "1", "3", "6"]
    );
    assert_eq!(points["(returned)"].len(), 1);
    let io = log.io("data:load").unwrap();
    assert_eq!(io.total, 2);
    assert_eq!(
        io.calls
            .iter()
            .map(|c| (c.input["n"].repr.as_deref().unwrap(), c.output.as_ref().unwrap().shape.clone().unwrap()))
            .collect::<Vec<_>>(),
        vec![("10", vec![10, 2]), ("3", vec![3, 2])]
    );
    assert_eq!(log.io("data:pipeline").unwrap().calls[0].output.as_ref().unwrap().kind, "dataframe");
}
#[test]
fn real_pandas_filter_group_and_merge_capture_shapes_columns_types_and_rows() {
    let Some(python) = forge_run::python::find_python(std::path::Path::new("."), None) else {
        return;
    };
    if !forge_sys::process::run_captured(
        &python,
        &["-c".into(), "import pandas".into()],
        std::path::Path::new("."),
        &[],
        "",
        Duration::from_secs(15),
    )
    .ok()
    {
        eprintln!("Optional pandas unavailable; skipping its real-library case.");
        return;
    }
    let source="import pandas as pd\ndef pipeline():\n    orders = pd.DataFrame({'region': ['N', 'N', 'S'], 'amount': [10, 20, 5]})\n    paid = orders[orders['amount'] >= 10]\n    grouped = paid.groupby('region', as_index=False).agg({'amount': 'sum'})\n    merged = grouped.merge(pd.DataFrame({'region': ['N'], 'label': ['North']}), on='region')\n    return merged\npipeline()\n";
    let mut log = DataLog::default();
    for m in run(source).unwrap() {
        log.add(m);
    }
    let at = |line, name: &str| {
        log.between("data:pipeline", line, line).into_iter().find(|p| p.var == name).unwrap().summary.clone()
    };
    let orders = at(3, "orders");
    let paid = at(4, "paid");
    let grouped = at(5, "grouped");
    let merged = at(6, "merged");
    assert_eq!(orders.shape.as_deref(), Some([3, 2].as_slice()));
    assert_eq!(paid.shape.as_deref(), Some([2, 2].as_slice()));
    assert_eq!(grouped.shape.as_deref(), Some([1, 2].as_slice()));
    assert_eq!(merged.shape.as_deref(), Some([1, 3].as_slice()));
    assert_eq!(
        merged.columns.as_ref().unwrap().iter().map(|c| c.0.as_str()).collect::<Vec<_>>(),
        ["region", "amount", "label"]
    );
    assert!(!merged.columns.as_ref().unwrap()[1].1.is_empty());
    assert_eq!(merged.rows.as_ref().unwrap()[0], ["'N'", "30", "'North'"]);
    assert_eq!(change(Some(&orders), Some(&paid), "filter"), "3 → 2 rows (−1)");
    assert_eq!(change(Some(&paid), Some(&grouped), "group"), "2 rows → 1 group");
    assert_eq!(change(Some(&grouped), Some(&merged), "merge"), "+1 column: label");
}
