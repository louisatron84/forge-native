//! The Tests panel: every test in the project, grouped by file, with its last
//! result. Run all, only the failed ones, a file or a single test (▶ on each
//! row, or Run Test at Cursor). Clicking a test opens it; clicking a failure
//! opens the line where it failed. Uses pytest when installed (unittest
//! TestCase tests run under it too), otherwise unittest. Replaces
//! `src/renderer/testView.ts`.

use crate::theme::{ForgeTheme, Theme, CODE_FONT};
use forge_core::output::last_lines;
use forge_core::testing::{
    failure_line, match_junit, parse_junit, parse_unittest, pytest_id, unittest_id, unittest_modules, DiscoveredTest,
    TestFile, TestResult, TestStatus,
};
use forge_run::tests::{discover_tests, has_pytest, TestRun, TestRunResult};
use futures::StreamExt;
use gpui_kit::{
    div, prelude::FluentBuilder, px, AnyElement, App, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, ParentElement, Render, ScrollHandle, SharedString, StatefulInteractiveElement,
    Styled, Task, Window,
};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// Lines of raw output kept in the Log view.
const MAX_LOG_LINES: usize = 5000;

/// Which tests to run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Which {
    All,
    Failed,
    These(Vec<String>),
}

pub enum TestsEvent {
    Open(PathBuf, usize),
    /// Run these (the workspace saves files and finds the interpreter first).
    Run(Which),
    InstallPytest,
    Stop,
}

impl EventEmitter<TestsEvent> for TestsPanel {}

pub struct TestsPanel {
    focus: FocusHandle,
    root: Option<PathBuf>,
    files: Vec<TestFile>,
    /// Last result per test, keyed by pytest id.
    results: HashMap<String, TestResult>,
    /// Tests in the run that's going on (pytest ids).
    running: Option<HashSet<String>>,
    summary: String,
    framework: Option<&'static str>,
    expanded: HashSet<String>,
    discovered: bool,
    pytest_missing: bool,
    show_log: bool,
    log: String,
    run: Option<TestRun>,
    scroll: ScrollHandle,
    _task: Option<Task<()>>,
}

impl Focusable for TestsPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

fn icon(status: Option<TestStatus>, running: bool) -> &'static str {
    if running {
        return "⟳";
    }
    match status {
        Some(TestStatus::Passed) => "✓",
        Some(TestStatus::Failed) => "✗",
        Some(TestStatus::Error) => "!",
        Some(TestStatus::Skipped) => "○",
        Some(TestStatus::NotRun) | None => "·",
    }
}

impl TestsPanel {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            root: None,
            files: Vec::new(),
            results: HashMap::new(),
            running: None,
            summary: String::new(),
            framework: None,
            expanded: HashSet::new(),
            discovered: false,
            pytest_missing: false,
            show_log: false,
            log: String::new(),
            run: None,
            scroll: ScrollHandle::new(),
            _task: None,
        }
    }

    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }

    /// Another folder was opened: forget everything.
    pub fn set_root(&mut self, root: Option<PathBuf>, cx: &mut Context<Self>) {
        self.stop();
        *self = Self { focus: self.focus.clone(), root, ..Self::new(cx) };
        cx.notify();
    }

    /// The panel is shown: find tests the first time.
    pub fn shown(&mut self, cx: &mut Context<Self>) {
        if !self.discovered && self._task.is_none() {
            self.discover(cx);
        }
    }

    /// (failed, total) for the panel's tab title.
    pub fn counts(&self) -> (usize, usize) {
        let ids = self.all_ids();
        let failed = ids
            .iter()
            .filter(|id| {
                matches!(self.results.get(*id).map(|r| r.status), Some(TestStatus::Failed | TestStatus::Error))
            })
            .count();
        (failed, ids.len())
    }

    fn all_ids(&self) -> Vec<String> {
        self.files
            .iter()
            .flat_map(|f| f.tests.iter().map(|t| pytest_id(&f.rel, &t.name, t.class_name.as_deref())))
            .collect()
    }

    /// Read the test files again (after opening a folder or saving a test file).
    pub fn discover(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else { return };
        self._task = Some(cx.spawn(async move |this, cx| {
            let r = root.clone();
            let files = cx.background_executor().spawn(async move { discover_tests(&r) }).await;
            this.update(cx, |p, cx| {
                if p.root.as_ref() != Some(&root) {
                    return;
                }
                p.files = files;
                p.discovered = true;
                // Drop results of tests that no longer exist.
                let ids: HashSet<String> = p.all_ids().into_iter().collect();
                p.results.retain(|id, _| ids.contains(id));
                p._task = None;
                cx.notify();
            })
            .ok();
        }));
    }

    fn file_for(&self, path: &str) -> Option<&TestFile> {
        let norm = |p: &str| p.replace('\\', "/").to_lowercase();
        self.files.iter().find(|f| norm(&f.path) == norm(path))
    }

    /// The test whose `def` contains this line (or the whole file), as ids.
    pub fn ids_at(&self, path: &str, line: Option<usize>) -> Option<Vec<String>> {
        let f = self.file_for(path)?;
        let all: Vec<String> = f.tests.iter().map(|t| pytest_id(&f.rel, &t.name, t.class_name.as_deref())).collect();
        let Some(line) = line else { return Some(all) };
        let test = f.tests.iter().filter(|t| t.line <= line).max_by_key(|t| t.line);
        Some(match test {
            Some(t) => vec![pytest_id(&f.rel, &t.name, t.class_name.as_deref())],
            None => all,
        })
    }

    pub fn say(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.summary = text.into();
        cx.notify();
    }

    fn tests_for(&self, ids: &[String]) -> Vec<(TestFile, DiscoveredTest)> {
        let set: HashSet<&String> = ids.iter().collect();
        self.files
            .iter()
            .flat_map(|f| {
                f.tests
                    .iter()
                    .filter(|t| set.contains(&pytest_id(&f.rel, &t.name, t.class_name.as_deref())))
                    .map(move |t| (f.clone(), t.clone()))
            })
            .collect()
    }

    /// A whole file selected: its path instead of every test in it (a shorter command line).
    fn shorten(&self, ids: &[String]) -> Vec<String> {
        let mut set: Vec<String> = ids.to_vec();
        let mut out = Vec::new();
        for f in &self.files {
            let file_ids: Vec<String> =
                f.tests.iter().map(|t| pytest_id(&f.rel, &t.name, t.class_name.as_deref())).collect();
            if file_ids.iter().all(|id| set.contains(id)) {
                out.push(f.rel.clone());
                set.retain(|id| !file_ids.contains(id));
            }
        }
        out.extend(set);
        out
    }

    /// Run tests with `python` (files are saved and tests discovered by now).
    pub fn run(&mut self, which: Which, python: String, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else { return self.say("Open a folder first.", cx) };
        if self.running.is_some() {
            return;
        }
        let all = which == Which::All;
        let ids: Vec<String> = match which {
            Which::All => self.all_ids(),
            Which::Failed => {
                let f: Vec<String> = self
                    .all_ids()
                    .into_iter()
                    .filter(|id| {
                        matches!(self.results.get(id).map(|r| r.status), Some(TestStatus::Failed | TestStatus::Error))
                    })
                    .collect();
                if f.is_empty() {
                    return self.say("No failed tests to run again.", cx);
                }
                f
            }
            Which::These(ids) => ids,
        };
        if self.files.is_empty() {
            return self
                .say("No tests found. Test files are named test_*.py or *_test.py, with functions named test_*.", cx);
        }
        self.pytest_missing = false;
        self.summary = "Checking for pytest…".into();
        cx.notify();
        let selected = self.tests_for(&ids);
        self._task = Some(cx.spawn(async move |this, cx| {
            let py = python.clone();
            let pytest = cx.background_executor().spawn(async move { has_pytest(&py) }).await;
            let (tx, mut rx) = futures::channel::mpsc::unbounded::<Result<String, TestRunResult>>();
            let started = this.update(cx, |p, cx| {
                if !pytest && selected.iter().any(|(_, t)| !t.unittest) {
                    p.summary.clear();
                    p.pytest_missing = true;
                    cx.notify();
                    return false;
                }
                p.framework = Some(if pytest { "pytest" } else { "unittest" });
                let run_ids: Vec<String> = if pytest {
                    if all {
                        Vec::new()
                    } else {
                        p.shorten(&ids)
                    }
                } else if all {
                    unittest_modules(&p.files)
                } else {
                    selected.iter().map(|(f, t)| unittest_id(&f.rel, &t.name, t.class_name.as_deref())).collect()
                };
                p.running = Some(ids.iter().cloned().collect());
                p.log = format!(
                    "$ {} {}   (in {})\n",
                    if pytest { "python -m pytest" } else { "python -m unittest -v" },
                    run_ids.join(" "),
                    root.display()
                );
                p.summary = format!(
                    "Running {} test{} with {}…",
                    ids.len(),
                    if ids.len() == 1 { "" } else { "s" },
                    p.framework.unwrap_or("")
                );
                let (tx1, tx2) = (tx.clone(), tx.clone());
                p.run = Some(TestRun::start(
                    &python,
                    &root,
                    pytest,
                    &run_ids,
                    move |text| {
                        let _ = tx1.unbounded_send(Ok(text));
                    },
                    move |result| {
                        let _ = tx2.unbounded_send(Err(result));
                    },
                ));
                cx.notify();
                true
            });
            drop(tx);
            if !matches!(started, Ok(true)) {
                return;
            }
            while let Some(msg) = rx.next().await {
                let done = this.update(cx, |p, cx| match msg {
                    Ok(text) => {
                        p.log.push_str(&text);
                        if p.log.len() > 4_000_000 {
                            p.log = last_lines(&p.log, MAX_LOG_LINES).0.to_owned();
                        }
                        cx.notify();
                        false
                    }
                    Err(result) => {
                        p.finish(&ids, result, cx);
                        true
                    }
                });
                if !matches!(done, Ok(false)) {
                    break;
                }
            }
        }));
    }

    fn finish(&mut self, ids: &[String], res: TestRunResult, cx: &mut Context<Self>) {
        self.running = None;
        self.run = None;
        self.log = last_lines(&self.log, MAX_LOG_LINES).0.to_owned();
        let mut got: HashMap<String, TestResult> = HashMap::new();
        match (self.framework, &res.xml) {
            (Some("pytest"), Some(xml)) => got = match_junit(&self.files, &parse_junit(xml)),
            (Some("unittest"), _) => {
                let by_id = parse_unittest(&res.output);
                for f in &self.files {
                    for t in &f.tests {
                        if let Some(r) = by_id.get(&unittest_id(&f.rel, &t.name, t.class_name.as_deref())) {
                            got.insert(pytest_id(&f.rel, &t.name, t.class_name.as_deref()), r.clone());
                        }
                    }
                }
            }
            _ => {}
        }
        for id in ids {
            let r = got.get(id).cloned().unwrap_or(TestResult {
                status: TestStatus::NotRun,
                message: String::new(),
                details: String::new(),
                seconds: 0.0,
            });
            self.results.insert(id.clone(), r);
        }
        for (id, r) in &got {
            self.results.insert(id.clone(), r.clone());
        }
        let mut counts: HashMap<TestStatus, usize> = HashMap::new();
        for id in ids {
            if let Some(r) = self.results.get(id) {
                *counts.entry(r.status).or_default() += 1;
            }
        }
        let c = |s| counts.get(&s).copied().unwrap_or(0);
        let parts: Vec<String> = [
            (c(TestStatus::Passed), "passed"),
            (c(TestStatus::Failed), "failed"),
            (c(TestStatus::Error), "errors"),
            (c(TestStatus::Skipped), "skipped"),
            (c(TestStatus::NotRun), "not run"),
        ]
        .into_iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, w)| format!("{n} {w}"))
        .collect();
        self.summary = if let Some(e) = &res.error {
            format!("{e}. See Log.")
        } else if got.is_empty() {
            let lines: Vec<&str> = res.output.trim().lines().collect();
            let tail: String = lines[lines.len().saturating_sub(2)..].join(" ").chars().take(200).collect();
            format!("No results came back{}. {tail} (see Log)", if res.exit_code.is_none() { " (stopped)" } else { "" })
        } else {
            format!("{} in {:.1}s ({})", parts.join(", "), res.seconds, self.framework.unwrap_or(""))
        };
        // Open the details of the first failure.
        if let Some(first) = ids
            .iter()
            .find(|id| matches!(self.results.get(*id).map(|r| r.status), Some(TestStatus::Failed | TestStatus::Error)))
        {
            self.expanded.insert(first.clone());
        }
        cx.notify();
    }

    pub fn stop(&mut self) {
        if let Some(r) = &self.run {
            r.stop();
        }
    }

    fn status_of(&self, id: &str) -> (Option<TestStatus>, bool) {
        (self.results.get(id).map(|r| r.status), self.running.as_ref().is_some_and(|r| r.contains(id)))
    }

    fn status_color(t: &Theme, status: Option<TestStatus>, running: bool) -> gpui_kit::Hsla {
        if running {
            return t.accent;
        }
        match status {
            Some(TestStatus::Passed) => t.ok,
            Some(TestStatus::Failed | TestStatus::Error) => t.err,
            _ => t.fg_muted,
        }
    }

    fn run_button(&self, id: SharedString, t: &Theme, which: Which, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id(id)
            .ml_auto()
            .px(px(6.0))
            .text_color(t.fg_muted)
            .cursor_pointer()
            .hover(|s| s.text_color(t.heat_2))
            .on_click(cx.listener(move |_, _, _, cx| {
                cx.stop_propagation();
                cx.emit(TestsEvent::Run(which.clone()));
            }))
            .child("▶")
    }

    fn render_file(&self, fi: usize, f: &TestFile, t: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let ids: Vec<String> = f.tests.iter().map(|x| pytest_id(&f.rel, &x.name, x.class_name.as_deref())).collect();
        let statuses: Vec<(Option<TestStatus>, bool)> = ids.iter().map(|id| self.status_of(id)).collect();
        let running = statuses.iter().any(|s| s.1);
        let failed = statuses.iter().any(|s| matches!(s.0, Some(TestStatus::Failed | TestStatus::Error)));
        let passed = !statuses.is_empty()
            && statuses.iter().all(|s| matches!(s.0, Some(TestStatus::Passed | TestStatus::Skipped)))
            && statuses.iter().any(|s| s.0 == Some(TestStatus::Passed));
        let file_status = if failed {
            Some(TestStatus::Failed)
        } else if passed {
            Some(TestStatus::Passed)
        } else {
            None
        };
        let (path, line0) = (PathBuf::from(&f.path), f.tests.first().map_or(0, |x| x.line));
        let mut out = vec![div()
            .id(("tests-file", fi))
            .flex()
            .items_center()
            .gap(px(6.0))
            .px(px(8.0))
            .py(px(3.0))
            .cursor_pointer()
            .hover(|s| s.bg(t.bg_hover))
            .on_click(cx.listener(move |_, _, _, cx| cx.emit(TestsEvent::Open(path.clone(), line0))))
            .child(
                div()
                    .w(px(14.0))
                    .text_color(Self::status_color(t, file_status, running))
                    .child(icon(file_status, running)),
            )
            .child(div().font_weight(gpui_kit::FontWeight::MEDIUM).child(f.rel.clone()))
            .child(div().text_size(px(11.0)).text_color(t.fg_muted).child(f.tests.len().to_string()))
            .child(self.run_button(
                SharedString::from(format!("tests-run-file-{fi}")),
                t,
                Which::These(ids.clone()),
                cx,
            ))
            .into_any_element()];
        for (ti, test) in f.tests.iter().enumerate() {
            let id = ids[ti].clone();
            let (status, running) = self.status_of(&id);
            let result = self.results.get(&id);
            let label = match &test.class_name {
                Some(c) => format!("{c} › {}", test.name),
                None => test.name.clone(),
            };
            let (path, line) = (PathBuf::from(&f.path), test.line);
            let time = result.filter(|r| r.seconds >= 0.01).map(|r| {
                if r.seconds < 1.0 {
                    format!("{}ms", (r.seconds * 1000.0).round())
                } else {
                    format!("{:.1}s", r.seconds)
                }
            });
            out.push(
                div()
                    .id(SharedString::from(format!("tests-row-{fi}-{ti}")))
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .pl(px(24.0))
                    .pr(px(8.0))
                    .py(px(2.0))
                    .cursor_pointer()
                    .hover(|s| s.bg(t.bg_hover))
                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(TestsEvent::Open(path.clone(), line))))
                    .child(
                        div()
                            .w(px(14.0))
                            .text_color(Self::status_color(t, status, running))
                            .child(icon(status, running)),
                    )
                    .child(label)
                    .when_some(time, |d, time| d.child(div().text_size(px(11.0)).text_color(t.fg_muted).child(time)))
                    .child(self.run_button(
                        SharedString::from(format!("tests-run-{fi}-{ti}")),
                        t,
                        Which::These(vec![id.clone()]),
                        cx,
                    ))
                    .into_any_element(),
            );
            let Some(r) = result else { continue };
            let show = matches!(r.status, TestStatus::Failed | TestStatus::Error)
                || (r.status == TestStatus::Skipped && !r.message.is_empty());
            if !show {
                continue;
            }
            let fail_at = failure_line(&r.details, &f.rel);
            let open = self.expanded.contains(&id);
            let text = if r.message.is_empty() {
                format!(
                    "{}{}",
                    if r.status == TestStatus::Error { "error" } else { "failed" },
                    fail_at.map_or(String::new(), |l| format!(" at line {}", l + 1))
                )
            } else {
                r.message.clone()
            };
            let (path, go) = (PathBuf::from(&f.path), fail_at.unwrap_or(test.line));
            let id2 = id.clone();
            out.push(
                div()
                    .id(SharedString::from(format!("tests-msg-{fi}-{ti}")))
                    .flex()
                    .gap(px(6.0))
                    .pl(px(44.0))
                    .pr(px(8.0))
                    .py(px(1.0))
                    .text_size(px(12.0))
                    .text_color(if r.status == TestStatus::Skipped { t.fg_muted } else { t.err })
                    .cursor_pointer()
                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(TestsEvent::Open(path.clone(), go))))
                    .when(!r.details.is_empty(), |d| {
                        d.child(
                            div()
                                .id(SharedString::from(format!("tests-toggle-{fi}-{ti}")))
                                .on_click(cx.listener(move |p, _, _, cx| {
                                    cx.stop_propagation();
                                    if !p.expanded.remove(&id2) {
                                        p.expanded.insert(id2.clone());
                                    }
                                    cx.notify();
                                }))
                                .child(if open { "▾" } else { "▸" }),
                        )
                    })
                    .child(div().min_w_0().overflow_hidden().text_ellipsis().whitespace_nowrap().child(text))
                    .into_any_element(),
            );
            if open && !r.details.is_empty() {
                out.push(
                    div()
                        .ml(px(44.0))
                        .mr(px(8.0))
                        .my(px(2.0))
                        .p(px(6.0))
                        .bg(t.bg_panel)
                        .font_family(CODE_FONT)
                        .text_size(px(12.0))
                        .children(r.details.lines().map(|l| div().whitespace_nowrap().child(l.replace('\t', "    "))))
                        .into_any_element(),
                );
            }
        }
        out
    }
}

/// The buttons in the Tests tab strip.
pub fn actions(panel: &Entity<TestsPanel>, t: &Theme) -> AnyElement {
    let button = |id: &'static str, label: &'static str, run: fn(&mut TestsPanel, &mut Context<TestsPanel>)| {
        let panel = panel.clone();
        div()
            .id(id)
            .px(px(6.0))
            .rounded(px(2.0))
            .text_size(px(12.0))
            .text_color(t.fg_muted)
            .cursor_pointer()
            .hover(|s| s.bg(t.bg_hover).text_color(t.fg))
            .on_click(move |_, _, cx| panel.update(cx, run))
            .child(label)
    };
    div()
        .flex()
        .gap(px(2.0))
        .child(button("tests-all", "▶ All", |_, cx| cx.emit(TestsEvent::Run(Which::All))))
        .child(button("tests-failed", "▶ Failed", |_, cx| cx.emit(TestsEvent::Run(Which::Failed))))
        .child(button("tests-rediscover", "⟳", |p, cx| p.discover(cx)))
        .child(button("tests-stop", "■", |_, cx| cx.emit(TestsEvent::Stop)))
        .child(button("tests-log", "Log", |p, cx| {
            p.show_log = !p.show_log;
            cx.notify();
        }))
        .into_any_element()
}

impl Render for TestsPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.global::<ForgeTheme>().0.clone();
        let (_, total) = self.counts();
        let summary = if !self.summary.is_empty() {
            self.summary.clone()
        } else if total > 0 {
            format!("{total} test{} found. ▶ All to run them.", if total == 1 { "" } else { "s" })
        } else {
            String::new()
        };
        let body: Vec<AnyElement> = if self.show_log {
            vec![div()
                .font_family(CODE_FONT)
                .text_size(px(12.0))
                .px(px(8.0))
                .children(self.log.lines().map(|l| div().whitespace_nowrap().child(l.replace('\t', "    "))))
                .into_any_element()]
        } else if self.root.is_none() {
            vec![div().p(px(10.0)).text_color(t.fg_muted).child("Open a folder to see its tests.").into_any_element()]
        } else if self.pytest_missing {
            vec![div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .p(px(10.0))
                .text_color(t.fg_muted)
                .child("These tests need pytest, which isn't installed in this interpreter.")
                .child(
                    div()
                        .id("tests-install-pytest")
                        .px(px(8.0))
                        .py(px(2.0))
                        .border_1()
                        .border_color(t.border)
                        .text_color(t.fg)
                        .cursor_pointer()
                        .hover(|s| s.bg(t.bg_hover))
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(TestsEvent::InstallPytest)))
                        .child("Install pytest"),
                )
                .into_any_element()]
        } else if self.files.is_empty() {
            let text = if self.discovered {
                "No tests found. Test files are named test_*.py or *_test.py, with functions named test_*."
            } else {
                "Looking for tests…"
            };
            vec![div().p(px(10.0)).text_color(t.fg_muted).child(text).into_any_element()]
        } else {
            let files = self.files.clone();
            files.iter().enumerate().flat_map(|(i, f)| self.render_file(i, f, &t, cx)).collect()
        };
        div()
            .id("tests-panel")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .text_size(px(13.0))
            .text_color(t.fg)
            .when(!summary.is_empty(), |d| {
                d.child(
                    div()
                        .flex_none()
                        .px(px(10.0))
                        .py(px(4.0))
                        .border_b_1()
                        .border_color(t.border)
                        .text_color(t.fg_muted)
                        .text_size(px(12.0))
                        .child(summary),
                )
            })
            .child(
                div()
                    .id("tests-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .py(px(4.0))
                    .children(body),
            )
    }
}
