//! Finding and reading the results of Python tests (pytest and unittest).
//! Discovery reads test files directly (test_*.py / *_test.py): top-level
//! `def test_*` functions and `test_*` methods of `Test*` classes or
//! unittest.TestCase subclasses (what pytest collects by default), with line
//! numbers for jumping to a test. Results come from pytest's JUnit XML
//! report, or unittest's -v output. Replaces `src/shared/tests.ts`.

use regex::Regex;
use std::collections::HashMap;
use std::sync::LazyLock;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredTest {
    pub name: String,
    /// The class it's in, or None for a top-level function.
    pub class_name: Option<String>,
    /// 0-based line of the `def`.
    pub line: usize,
    /// In a unittest.TestCase class (so unittest can run it too).
    pub unittest: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestFile {
    pub path: String,
    /// Relative to the project, with / separators.
    pub rel: String,
    pub tests: Vec<DiscoveredTest>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TestStatus {
    NotRun,
    Skipped,
    Passed,
    Failed,
    Error,
}

impl TestStatus {
    fn rank(self) -> u8 {
        match self {
            TestStatus::NotRun => 0,
            TestStatus::Skipped => 1,
            TestStatus::Passed => 2,
            TestStatus::Failed => 3,
            TestStatus::Error => 4,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TestResult {
    pub status: TestStatus,
    /// One-line reason (assertion message, skip reason).
    pub message: String,
    /// Full traceback / details.
    pub details: String,
    pub seconds: f64,
}

/// Is this (relative) path a test file pytest would collect by default?
pub fn is_test_file(rel: &str) -> bool {
    let name = rel.rsplit(['/', '\\']).next().unwrap_or(rel);
    name.ends_with(".py")
        && (name.starts_with("test_") || (name.ends_with("_test.py") && name.len() > "_test.py".len()))
}

static CLASS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\s*)class\s+([A-Za-z_]\w*)\s*(\(([^)]*)\))?\s*:").expect("regex"));
static DEF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\s*)(?:async\s+)?def\s+(test\w*)\s*\(").expect("regex"));
static ANY_DEF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*(async\s+)?def\s").expect("regex"));
static TRIPLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"^\s*[rRbBuUfF]*("""|''')"#).expect("regex"));
static UNITTEST_BASE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bTestCase\b|\bIsolatedAsyncioTestCase\b").expect("regex"));

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

fn blank_or_comment(line: &str) -> bool {
    let t = line.trim();
    t.is_empty() || t.starts_with('#')
}

/// A def directly in the class body (not nested in another method).
fn is_method_level(lines: &[&str], at: usize, indent: usize, class_indent: usize) -> bool {
    for l in lines[..at].iter().rev() {
        if blank_or_comment(l) {
            continue;
        }
        let ind = indent_of(l);
        if ind <= class_indent {
            return true; // reached the class line
        }
        if ind < indent {
            return !ANY_DEF.is_match(l);
        }
    }
    true
}

/// Tests in a Python source file.
pub fn find_tests(source: &str) -> Vec<DiscoveredTest> {
    struct Cls {
        name: String,
        indent: usize,
        collect: bool,
        unittest: bool,
    }
    let mut out = Vec::new();
    let lines: Vec<&str> = source.lines().collect();
    let mut cls: Option<Cls> = None;
    let mut in_string: Option<&str> = None;
    for (i, line) in lines.iter().enumerate() {
        // Skip the insides of triple-quoted strings (docstrings), roughly.
        if let Some(q) = in_string {
            if line.contains(q) {
                in_string = None;
            }
            continue;
        }
        if let Some(c) = TRIPLE.captures(line) {
            let q = if &c[1] == "\"\"\"" { "\"\"\"" } else { "'''" };
            if line.matches(q).count() == 1 {
                in_string = Some(q);
                continue;
            }
        }
        if blank_or_comment(line) {
            continue;
        }
        let indent = indent_of(line);
        if cls.as_ref().is_some_and(|c| indent <= c.indent) {
            cls = None;
        }
        if let Some(c) = CLASS.captures(line) {
            if c[1].is_empty() {
                let bases = c.get(4).map_or("", |m| m.as_str());
                let unittest = UNITTEST_BASE.is_match(bases);
                // pytest collects Test* classes; unittest needs TestCase.
                cls = Some(Cls {
                    name: c[2].to_owned(),
                    indent: 0,
                    collect: unittest || c[2].starts_with("Test"),
                    unittest,
                });
            }
            continue;
        }
        let Some(d) = DEF.captures(line) else { continue };
        let def_indent = d[1].len();
        if def_indent == 0 {
            out.push(DiscoveredTest { name: d[2].to_owned(), class_name: None, line: i, unittest: false });
        } else if let Some(c) = &cls {
            if c.collect && def_indent > c.indent && is_method_level(&lines, i, def_indent, c.indent) {
                out.push(DiscoveredTest {
                    name: d[2].to_owned(),
                    class_name: Some(c.name.clone()),
                    line: i,
                    unittest: c.unittest,
                });
            }
        }
    }
    out
}

/// pytest's id for a test: path/to/test_x.py::Class::test_name.
pub fn pytest_id(rel: &str, name: &str, class_name: Option<&str>) -> String {
    let rel = rel.replace('\\', "/");
    [Some(rel.as_str()), class_name, Some(name)].into_iter().flatten().collect::<Vec<_>>().join("::")
}

fn module_of(rel: &str) -> String {
    let r = rel.replace('\\', "/");
    r.strip_suffix(".py").unwrap_or(&r).split('/').collect::<Vec<_>>().join(".")
}

/// unittest's id: path.to.test_x.Class.test_name (only for TestCase methods).
pub fn unittest_id(rel: &str, name: &str, class_name: Option<&str>) -> String {
    let m = module_of(rel);
    [Some(m.as_str()), class_name, Some(name)].into_iter().flatten().collect::<Vec<_>>().join(".")
}

/// The modules to give unittest for "run all": every test file by module name
/// (unittest discover only looks in folders with an __init__.py).
pub fn unittest_modules(files: &[TestFile]) -> Vec<String> {
    files.iter().filter(|f| f.tests.iter().any(|t| t.unittest)).map(|f| module_of(&f.rel)).collect()
}

fn unescape_xml(s: &str) -> String {
    static ENT: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"&(lt|gt|quot|apos|amp|#(\d+)|#x([0-9a-fA-F]+));").expect("regex"));
    ENT.replace_all(s, |c: &regex::Captures| {
        if let Some(d) = c.get(2) {
            return d.as_str().parse::<u32>().ok().and_then(char::from_u32).map(String::from).unwrap_or_default();
        }
        if let Some(h) = c.get(3) {
            return u32::from_str_radix(h.as_str(), 16)
                .ok()
                .and_then(char::from_u32)
                .map(String::from)
                .unwrap_or_default();
        }
        match &c[1] {
            "lt" => "<",
            "gt" => ">",
            "quot" => "\"",
            "apos" => "'",
            _ => "&",
        }
        .to_owned()
    })
    .into_owned()
}

fn attrs(text: &str) -> HashMap<String, String> {
    static ATTR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"([\w:-]+)\s*=\s*"([^"]*)""#).expect("regex"));
    ATTR.captures_iter(text).map(|c| (c[1].to_owned(), unescape_xml(&c[2]))).collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct JUnitCase {
    pub classname: String,
    pub name: String,
    /// With -o junit_family=xunit1: the file (relative to pytest's rootdir) and 0-based line.
    pub file: Option<String>,
    pub line: Option<usize>,
    pub result: TestResult,
}

/// Read pytest's --junitxml report.
pub fn parse_junit(xml: &str) -> Vec<JUnitCase> {
    static CASE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?s)<testcase\b([^>]*?)(?:/>|>(.*?)</testcase>)").expect("regex"));
    static CHILD: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?s)<(failure|error|skipped)\b([^>]*?)(?:/>|>(.*?)</(?:failure|error|skipped)>)").expect("regex")
    });
    let mut out = Vec::new();
    for m in CASE.captures_iter(xml) {
        let a = attrs(&m[1]);
        let body = m.get(2).map_or("", |b| b.as_str());
        let (mut status, mut message, mut details) = (TestStatus::Passed, String::new(), String::new());
        if let Some(child) = CHILD.captures(body) {
            status = match &child[1] {
                "failure" => TestStatus::Failed,
                "error" => TestStatus::Error,
                _ => TestStatus::Skipped,
            };
            message = attrs(&child[2]).remove("message").unwrap_or_default();
            let raw = child.get(3).map_or("", |c| c.as_str());
            let raw = raw.strip_prefix("<![CDATA[").unwrap_or(raw);
            let raw = raw.strip_suffix("]]>").unwrap_or(raw);
            details = unescape_xml(raw);
        }
        let first: String = message.lines().next().unwrap_or("").chars().take(300).collect();
        out.push(JUnitCase {
            classname: a.get("classname").cloned().unwrap_or_default(),
            name: a.get("name").cloned().unwrap_or_default(),
            file: a.get("file").cloned(),
            line: a.get("line").and_then(|l| l.parse().ok()),
            result: TestResult {
                status,
                message: first,
                details,
                seconds: a.get("time").and_then(|t| t.parse().ok()).unwrap_or(0.0),
            },
        });
    }
    out
}

/// Read `python -m unittest -v` output: id (module.Class.test) -> result.
pub fn parse_unittest(output: &str) -> HashMap<String, TestResult> {
    static LINE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?m)^(\w+) \(([\w.]+)\)(?:\n[^\n]*?)? \.\.\. (ok|FAIL|ERROR|skipped(?: '(.*)')?|expected failure|unexpected success)\s*$").expect("regex")
    });
    static SEP_EQ: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^={20,}\r?$").expect("regex"));
    static HEAD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(FAIL|ERROR): (\w+) \(([\w.]+)\)").expect("regex"));
    static SEP_DASH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^-{20,}\r?$").expect("regex"));
    let id_of = |name: &str, paren: &str| {
        if paren.ends_with(&format!(".{name}")) {
            paren.to_owned()
        } else {
            format!("{paren}.{name}")
        }
    };
    let mut results = HashMap::new();
    for m in LINE.captures_iter(output) {
        let word = &m[3];
        let status = match word {
            "ok" | "expected failure" => TestStatus::Passed,
            "FAIL" | "unexpected success" => TestStatus::Failed,
            "ERROR" => TestStatus::Error,
            _ => TestStatus::Skipped,
        };
        let message = m.get(4).map(|x| x.as_str().to_owned()).unwrap_or_else(|| {
            if word == "unexpected success" {
                "unexpected success".into()
            } else {
                String::new()
            }
        });
        results.insert(id_of(&m[1], &m[2]), TestResult { status, message, details: String::new(), seconds: 0.0 });
    }
    // Failure details: "=====\nFAIL: test_x (pkg.mod.Class.test_x)\n[docstring]\n-----\n<traceback>"
    let blocks: Vec<&str> = SEP_EQ.split(output).skip(1).collect();
    for block in blocks {
        let block = block.trim_start_matches(['\r', '\n']);
        let Some(h) = HEAD.captures(block) else { continue };
        let Some(dash) = SEP_DASH.find(block) else { continue };
        let mut details = &block[dash.end()..];
        details = details.trim_start_matches(['\r', '\n']);
        // The last block ends at "-----\nRan N tests".
        if let Some(end) = SEP_DASH.find(details) {
            details = &details[..end.start()];
        }
        let details = details.trim_end().to_owned();
        let last: String =
            details.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").chars().take(300).collect();
        let id = id_of(&h[2], &h[3]);
        let seconds = results.get(&id).map_or(0.0, |r: &TestResult| r.seconds);
        let status = if &h[1] == "FAIL" { TestStatus::Failed } else { TestStatus::Error };
        results.insert(id, TestResult { status, message: last, details, seconds });
    }
    results
}

/// Where a failure happened in `file`, from a traceback: the last
/// `File "...", line N` (unittest) or `path:N: Error` (pytest) in that file. 0-based.
pub fn failure_line(details: &str, file_ends_with: &str) -> Option<usize> {
    static FILE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"File "([^"]+)", line (\d+)"#).expect("regex"));
    static PYTEST: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^([^\s:][^:\n]*\.py):(\d+): ").expect("regex"));
    let want = file_ends_with.replace('\\', "/").to_lowercase();
    let matches = |p: &str| {
        let norm = p.replace('\\', "/").to_lowercase();
        norm == want || norm.ends_with(&format!("/{want}")) || want.ends_with(&format!("/{norm}"))
    };
    let mut found = None;
    for c in FILE.captures_iter(details) {
        if matches(&c[1]) {
            found = c[2].parse::<usize>().ok().map(|n| n.saturating_sub(1));
        }
    }
    for c in PYTEST.captures_iter(details) {
        if matches(&c[1]) {
            found = c[2].parse::<usize>().ok().map(|n| n.saturating_sub(1));
        }
    }
    found
}

/// Combine results of the same test (parametrized cases): the worst status wins.
fn merge(a: Option<&TestResult>, b: TestResult) -> TestResult {
    let Some(a) = a else { return b };
    let worst = if b.status.rank() > a.status.rank() { &b } else { a };
    let details =
        [a.details.as_str(), b.details.as_str()].into_iter().filter(|d| !d.is_empty()).collect::<Vec<_>>().join("\n\n");
    TestResult { status: worst.status, message: worst.message.clone(), details, seconds: a.seconds + b.seconds }
}

/// Match pytest's report to the discovered tests: pytest id -> result.
pub fn match_junit(files: &[TestFile], cases: &[JUnitCase]) -> HashMap<String, TestResult> {
    static PARAMS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)\[.*\]$").expect("regex"));
    let mut out: HashMap<String, TestResult> = HashMap::new();
    for c in cases {
        let name = PARAMS.replace(&c.name, "").into_owned();
        for f in files {
            let module = module_of(&f.rel);
            if let Some(file) = &c.file {
                let file = file.replace('\\', "/");
                if !(file == f.rel || f.rel.ends_with(&format!("/{file}")) || file.ends_with(&format!("/{}", f.rel))) {
                    continue;
                }
            } else {
                let parent: Vec<&str> = c.classname.split('.').collect();
                let parent = parent[..parent.len().saturating_sub(1)].join(".");
                if !(c.classname == module
                    || c.classname.starts_with(&format!("{module}."))
                    || module.ends_with(&format!(".{parent}")))
                {
                    continue;
                }
            }
            // classname is "module.path" for functions and "module.path.Class" for methods.
            let last = c.classname.rsplit('.').next().unwrap_or("");
            let module_name = module.rsplit('.').next().unwrap_or("");
            let Some(test) = f.tests.iter().find(|t| {
                t.name == name
                    && match &t.class_name {
                        None => last == module_name,
                        Some(cls) => cls == last,
                    }
            }) else {
                continue;
            };
            let id = pytest_id(&f.rel, &test.name, test.class_name.as_deref());
            let merged = merge(out.get(&id), c.result.clone());
            out.insert(id, merged);
            break;
        }
    }
    out
}

/// The command line for a test run (`discover` is a subcommand, so it comes before -v).
pub fn test_args(pytest: bool, ids: &[String], xml_path: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if pytest {
        out.extend(["-m", "pytest", "-o", "junit_family=xunit1"].map(String::from));
        out.push(format!("--junitxml={xml_path}"));
        out.extend(["--color=no", "-q"].map(String::from));
    } else if ids.is_empty() {
        out.extend(["-m", "unittest", "discover", "-v"].map(String::from));
    } else {
        out.extend(["-m", "unittest", "-v"].map(String::from));
    }
    out.extend(ids.iter().cloned());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(name: &str, class: Option<&str>, line: usize, unittest: bool) -> DiscoveredTest {
        DiscoveredTest { name: name.into(), class_name: class.map(Into::into), line, unittest }
    }

    #[test]
    fn matches_test_files() {
        assert!(is_test_file("tests/test_app.py"));
        assert!(is_test_file("app_test.py"));
        assert!(!is_test_file("src/app.py"));
        assert!(!is_test_file("test_data.json"));
    }

    #[test]
    fn finds_test_functions_and_methods_with_lines() {
        let src = [
            "import unittest",
            "",
            "def test_add():",
            "    def test_inner(): pass",
            "    assert 1 + 1 == 2",
            "",
            "def helper(): pass",
            "",
            "class TestMath:",
            "    \"\"\"Docs",
            "    def test_fake(): pass",
            "    \"\"\"",
            "    def test_mul(self):",
            "        pass",
            "    async def test_async(self):",
            "        pass",
            "",
            "class Helper:",
            "    def test_not_collected(self): pass",
            "",
            "class Cases(unittest.TestCase):",
            "    def test_eq(self):",
            "        self.assertEqual(1, 1)",
            "    def setUp(self): pass",
            "def test_last(): pass",
        ]
        .join("\n");
        assert_eq!(
            find_tests(&src),
            vec![
                t("test_add", None, 2, false),
                t("test_mul", Some("TestMath"), 12, false),
                t("test_async", Some("TestMath"), 14, false),
                t("test_eq", Some("Cases"), 21, true),
                t("test_last", None, 24, false),
            ]
        );
    }

    #[test]
    fn builds_ids() {
        assert_eq!(pytest_id("tests\\test_a.py", "test_x", Some("TestK")), "tests/test_a.py::TestK::test_x");
        assert_eq!(pytest_id("test_a.py", "test_x", None), "test_a.py::test_x");
        assert_eq!(unittest_id("tests/test_a.py", "test_x", Some("TestK")), "tests.test_a.TestK.test_x");
    }

    #[test]
    fn reads_junit_reports() {
        let xml = "<?xml version=\"1.0\" encoding=\"utf-8\"?><testsuites><testsuite name=\"pytest\" errors=\"0\" failures=\"1\">\n<testcase classname=\"tests.test_a\" name=\"test_ok\" file=\"tests/test_a.py\" line=\"2\" time=\"0.001\" />\n<testcase classname=\"tests.test_a.TestK\" name=\"test_bad[1]\" file=\"tests/test_a.py\" line=\"9\" time=\"0.002\"><failure message=\"assert 1 == 2&#10;more\">def test_bad():\n&gt;   assert 1 == 2\nE   assert 1 == 2\n\ntests/test_a.py:11: AssertionError</failure></testcase>\n<testcase classname=\"tests.test_a\" name=\"test_skip\" time=\"0\"><skipped type=\"pytest.skip\" message=\"not today\">skip text</skipped></testcase>\n<testcase classname=\"tests.test_a\" name=\"test_err\" time=\"0\"><error message=\"fixture &apos;db&apos; not found\" /></testcase>\n</testsuite></testsuites>";
        let cases = parse_junit(xml);
        type Row = (String, String, Option<String>, Option<usize>, TestStatus, String);
        let summary: Vec<Row> = cases
            .iter()
            .map(|c| {
                (c.classname.clone(), c.name.clone(), c.file.clone(), c.line, c.result.status, c.result.message.clone())
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                (
                    "tests.test_a".into(),
                    "test_ok".into(),
                    Some("tests/test_a.py".into()),
                    Some(2),
                    TestStatus::Passed,
                    String::new()
                ),
                (
                    "tests.test_a.TestK".into(),
                    "test_bad[1]".into(),
                    Some("tests/test_a.py".into()),
                    Some(9),
                    TestStatus::Failed,
                    "assert 1 == 2".into()
                ),
                ("tests.test_a".into(), "test_skip".into(), None, None, TestStatus::Skipped, "not today".into()),
                (
                    "tests.test_a".into(),
                    "test_err".into(),
                    None,
                    None,
                    TestStatus::Error,
                    "fixture 'db' not found".into()
                ),
            ]
        );
        assert!(cases[1].result.details.contains(">   assert 1 == 2"));
        assert_eq!(failure_line(&cases[1].result.details, "tests/test_a.py"), Some(10));
    }

    #[test]
    fn reads_unittest_output() {
        let out = "test_eq (tests.test_a.Cases.test_eq) ... ok\ntest_bad (tests.test_a.Cases)\nChecks something. ... FAIL\ntest_skip (tests.test_a.Cases.test_skip) ... skipped 'later'\ntest_boom (tests.test_a.Cases.test_boom) ... ERROR\n\n======================================================================\nFAIL: test_bad (tests.test_a.Cases)\nChecks something.\n----------------------------------------------------------------------\nTraceback (most recent call last):\n  File \"C:\\p\\tests\\test_a.py\", line 9, in test_bad\n    self.assertEqual(1, 2)\nAssertionError: 1 != 2\n\n======================================================================\nERROR: test_boom (tests.test_a.Cases.test_boom)\n----------------------------------------------------------------------\nTraceback (most recent call last):\n  File \"/p/tests/test_a.py\", line 14, in test_boom\n    raise ValueError(\"x\")\nValueError: x\n\n----------------------------------------------------------------------\nRan 4 tests in 0.002s\n\nFAILED (failures=1, errors=1, skipped=1)\n";
        let r = parse_unittest(out);
        assert_eq!(r.get("tests.test_a.Cases.test_eq").map(|x| x.status), Some(TestStatus::Passed));
        let skip = r.get("tests.test_a.Cases.test_skip").expect("skip");
        assert_eq!((skip.status, skip.message.as_str()), (TestStatus::Skipped, "later"));
        let boom = r.get("tests.test_a.Cases.test_boom").expect("boom");
        assert_eq!((boom.status, boom.message.as_str()), (TestStatus::Error, "ValueError: x"));
        let bad = r.get("tests.test_a.Cases.test_bad").expect("bad");
        assert_eq!(bad.status, TestStatus::Failed);
        assert_eq!(bad.message, "AssertionError: 1 != 2");
        assert_eq!(failure_line(&bad.details, "tests/test_a.py"), Some(8));
    }

    #[test]
    fn matches_junit_cases_merging_parametrized_ones() {
        let files = vec![TestFile {
            path: "/p/tests/test_a.py".into(),
            rel: "tests/test_a.py".into(),
            tests: vec![
                t("test_ok", None, 1, false),
                t("test_bad", Some("TestK"), 5, false),
                t("test_p", None, 9, false),
            ],
        }];
        let r = |status, message: &str| TestResult {
            status,
            message: message.into(),
            details: message.into(),
            seconds: 0.1,
        };
        let case = |classname: &str, name: &str, file: Option<&str>, result| JUnitCase {
            classname: classname.into(),
            name: name.into(),
            file: file.map(Into::into),
            line: None,
            result,
        };
        let m = match_junit(
            &files,
            &[
                case("tests.test_a", "test_ok", Some("tests/test_a.py"), r(TestStatus::Passed, "")),
                case("tests.test_a.TestK", "test_bad", Some("tests/test_a.py"), r(TestStatus::Failed, "boom")),
                case("tests.test_a", "test_p[1]", None, r(TestStatus::Passed, "")),
                case("tests.test_a", "test_p[2]", None, r(TestStatus::Failed, "p2")),
            ],
        );
        assert_eq!(m.get("tests/test_a.py::test_ok").map(|x| x.status), Some(TestStatus::Passed));
        assert_eq!(m.get("tests/test_a.py::TestK::test_bad").map(|x| x.message.as_str()), Some("boom"));
        let p = m.get("tests/test_a.py::test_p").expect("p");
        assert_eq!((p.status, p.message.as_str()), (TestStatus::Failed, "p2"));
    }

    #[test]
    fn names_unittest_modules() {
        let f = |rel: &str, unittest: bool| TestFile {
            path: String::new(),
            rel: rel.into(),
            tests: vec![t("x", None, 0, unittest)],
        };
        assert_eq!(
            unittest_modules(&[
                f("tests/test_a.py", true),
                f("tests\\sub\\test_b.py", true),
                f("test_plain.py", false)
            ]),
            ["tests.test_a", "tests.sub.test_b"]
        );
    }

    #[test]
    fn command_lines() {
        assert_eq!(test_args(false, &[], "x.xml"), ["-m", "unittest", "discover", "-v"]);
        assert_eq!(
            test_args(false, &["tests.test_a.T.test_x".into()], "x.xml"),
            ["-m", "unittest", "-v", "tests.test_a.T.test_x"]
        );
        assert_eq!(
            test_args(true, &["t.py::a".into()], "out.xml"),
            ["-m", "pytest", "-o", "junit_family=xunit1", "--junitxml=out.xml", "--color=no", "-q", "t.py::a"]
        );
    }
}
