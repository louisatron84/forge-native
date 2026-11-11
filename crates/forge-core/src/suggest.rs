//! Context-aware toolbar tools, replacing `src/shared/suggest.ts`.
//! File hints are cached by document version by the caller, so drawing the
//! toolbar never needs to scan an unchanged document again.

#[derive(Clone, Copy, Debug, Default)]
pub struct FileHints {
    pub tests: bool,
    pub web: bool,
    pub database: bool,
    pub tables: bool,
}

impl FileHints {
    pub fn scan(path: &str, text: &str) -> Self {
        use regex::Regex;
        use std::sync::LazyLock;
        // These are fixed patterns from the Electron version, not user input.
        static WEB: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"(?m)^\s*(import|from)\s+(requests|httpx|aiohttp|urllib\.request|urllib3)\b")
                .expect("fixed web regex")
        });
        static TABLES: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"(?m)^\s*(import|from)\s+(pandas|polars|forge_blocks)\b").expect("fixed tables regex")
        });
        static DB: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"(?m)^\s*(import|from)\s+(sqlite3|aiosqlite|sqlalchemy|sqlmodel|peewee|psycopg2?|asyncpg|pymysql|pymongo|redis|django\.db)\b").expect("fixed database regex")
        });
        static TEST: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"(?i)(^|[\\/])(test_[^\\/]*|[^\\/]*_test)\.py$").expect("fixed test path regex")
        });
        static TEST_DEF: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"(?m)^\s*def test_").expect("fixed test def regex"));
        Self {
            tests: TEST.is_match(path) || TEST_DEF.is_match(text),
            web: WEB.is_match(text),
            database: DB.is_match(text),
            tables: TABLES.is_match(text),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DebugStatus {
    #[default]
    Idle,
    Running,
    Paused,
}

pub struct WorkContext<'a> {
    pub task: &'a str,
    pub file: Option<FileHints>,
    pub debug: DebugStatus,
    pub last_run_failed: bool,
    pub problems: usize,
    pub changes: usize,
}

impl Default for WorkContext<'_> {
    fn default() -> Self {
        Self { task: "code", file: None, debug: DebugStatus::Idle, last_run_failed: false, problems: 0, changes: 0 }
    }
}

#[derive(Clone, Debug)]
pub struct Suggestion {
    pub id: &'static str,
    pub label: String,
    pub why: String,
    pub panel: Option<&'static str>,
    pub command: Option<&'static str>,
}

pub fn suggest_tools(ctx: &WorkContext<'_>, shown: impl Fn(&str) -> bool, max: usize) -> Vec<Suggestion> {
    let mut out = Vec::with_capacity(max.min(3));
    let mut add = |id, label: String, why: String, panel: Option<&'static str>, command| {
        if out.len() < max && !panel.is_some_and(&shown) {
            out.push(Suggestion { id, label, why, panel, command });
        }
    };
    if ctx.debug == DebugStatus::Paused {
        add("debug", "Debug".into(), "Paused: see the call stack and variables".into(), Some("debug"), None);
    }
    if ctx.last_run_failed && ctx.debug == DebugStatus::Idle {
        add(
            "debug-it",
            "Debug it".into(),
            "The last run failed: run it again in the debugger".into(),
            None,
            Some("debug.start"),
        );
    }
    if ctx.problems > 0 {
        add(
            "problems",
            format!("Problems ({})", ctx.problems),
            format!("{} problem{} in this file", ctx.problems, if ctx.problems == 1 { "" } else { "s" }),
            Some("problems"),
            None,
        );
    }
    if let Some(file) = ctx.file {
        for (enabled, id, label, why, panel) in [
            (file.tests, "tests", "Tests", "This file has tests: run them and see the results", "tests"),
            (file.web, "network", "Network", "This file calls web APIs: see each request while it runs", "network"),
            (
                file.database,
                "database",
                "Database",
                "This file uses a database: see its tables, keys and connections",
                "database",
            ),
            (
                file.tables,
                "pipeline",
                "Pipeline",
                "This file works with tables: see its steps and the data at each one",
                "pipeline",
            ),
        ] {
            if enabled {
                add(id, label.into(), why.into(), Some(panel), None);
            }
        }
    }
    if ctx.changes > 0 && ctx.task == "code" {
        add(
            "git",
            format!("Git ({})", ctx.changes),
            format!("{} changed file{} to commit", ctx.changes, if ctx.changes == 1 { "" } else { "s" }),
            Some("git"),
            None,
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ids(ctx: &WorkContext<'_>) -> Vec<&'static str> {
        suggest_tools(ctx, |_| false, 3).iter().map(|s| s.id).collect()
    }
    #[test]
    fn nothing_when_nothing_is_going_on() {
        assert!(ids(&WorkContext::default()).is_empty());
    }
    #[test]
    fn from_the_open_file_web_apis_tables_tests() {
        for (path, text, expected) in [
            ("app/api.py", "import requests\n", vec!["network"]),
            ("etl.py", "import pandas as pd\n", vec!["pipeline"]),
            ("tests/test_x.py", "from forge_blocks import read_csv\n", vec!["tests", "pipeline"]),
        ] {
            assert_eq!(ids(&WorkContext { file: Some(FileHints::scan(path, text)), ..Default::default() }), expected);
        }
    }
    #[test]
    fn a_file_that_uses_a_database_suggests_the_database_panel() {
        for text in ["import sqlite3\n", "from sqlalchemy.orm import Mapped\n", "from django.db import models\n"] {
            let ctx = WorkContext { file: Some(FileHints::scan("db.py", text)), ..Default::default() };
            assert_eq!(ids(&ctx), ["database"]);
            assert!(suggest_tools(&ctx, |p| p == "database", 3).is_empty());
        }
    }
    #[test]
    fn a_failed_run_suggests_debugging_it_a_pause_the_debug_panel() {
        let mut ctx = WorkContext { last_run_failed: true, ..Default::default() };
        assert_eq!(ids(&ctx), ["debug-it"]);
        ctx.debug = DebugStatus::Paused;
        assert_eq!(ids(&ctx), ["debug"]);
    }
    #[test]
    fn problems_and_changes_at_most_three_most_urgent_first_skip_visible_panels() {
        let ctx = WorkContext {
            problems: 2,
            changes: 5,
            last_run_failed: true,
            file: Some(FileHints::scan("a.py", "import httpx\nimport pandas\n")),
            ..Default::default()
        };
        assert_eq!(ids(&ctx), ["debug-it", "problems", "network"]);
        let suggestions = suggest_tools(&ctx, |p| matches!(p, "problems" | "network"), 3);
        assert_eq!(suggestions.iter().map(|s| s.id).collect::<Vec<_>>(), ["debug-it", "pipeline", "git"]);
        assert_eq!(suggest_tools(&ctx, |_| false, 3)[1].label, "Problems (2)");
        assert!(suggest_tools(&ctx, |_| false, 0).is_empty());
    }
}
