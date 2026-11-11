//! The panels the dock arranges (Files, Search, Terminal...): their titles,
//! and a stand-in for panels not ported to the native app yet.

pub mod connect;
pub mod debug;
pub mod files;
pub mod git;
pub mod git_inspect;
pub mod graph;
pub mod lsp_views;
pub mod network;
pub mod openapi;
pub mod output;
pub mod pipeline;
pub mod problems;
pub mod search;
pub mod terminal;
pub mod tests;

use crate::theme::ForgeTheme;
use gpui_kit::{div, Context, IntoElement, ParentElement, Render, SharedString, Styled, Window};

/// The title shown on a panel's tab.
pub fn title(id: &str) -> &'static str {
    match id {
        "files" => "Files",
        "search" => "Search",
        "outline" => "Outline",
        "git" => "Git",
        "history" => "History",
        "editor" => "Editor",
        "where" => "Where you are",
        "infile" => "In this file",
        "graph" => "Graph",
        "toolbox" => "Toolbox",
        "box" => "Box settings",
        "terminal" => "Terminal",
        "problems" => "Problems",
        "output" => "Output",
        "debug" => "Debug",
        "tests" => "Tests",
        "references" => "References",
        "calls" => "Calls",
        "network" => "Network",
        "pipeline" => "Pipeline",
        "database" => "Database",
        "memory" => "Memory",
        _ => "Panel",
    }
}

/// A panel that isn't in the native app yet.
pub struct Placeholder {
    pub title: SharedString,
}

impl Render for Placeholder {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.global::<ForgeTheme>().0.clone();
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .text_color(t.fg_muted)
            .text_size(t.px(12.0))
            .child(format!("{} is being ported to the native app.", self.title))
    }
}
