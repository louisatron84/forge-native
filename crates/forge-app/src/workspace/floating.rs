//! Panels in native OS windows, replacing `src/renderer/dock/floating.ts`.
//! Views keep their entities and subscriptions; layouts keep their placements
//! so closing a window or restarting never loses a panel's dock position.

use super::Workspace;
use crate::{panels, theme::ForgeTheme};
use forge_core::dock::{self, DockNode};
use gpui_kit::{
    div, px, AnyView, AppContext, Context, InteractiveElement, IntoElement, ParentElement, Render,
    StatefulInteractiveElement, Styled, WeakEntity, Window,
};

struct FloatingPanel {
    panel: &'static str,
    content: AnyView,
    owner: WeakEntity<Workspace>,
    output: gpui_kit::Entity<panels::output::OutputPanel>,
    tests: gpui_kit::Entity<panels::tests::TestsPanel>,
}

impl Render for FloatingPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.global::<ForgeTheme>().0.clone();
        let actions = match self.panel {
            "output" => Some(panels::output::actions(&self.output, &t)),
            "tests" => Some(panels::tests::actions(&self.tests, &t)),
            _ => None,
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(t.bg_panel)
            .text_color(t.fg)
            .font_family(crate::theme::UI_FONT)
            .text_size(t.px(13.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .h(t.px(32.0))
                    .flex_none()
                    .px(t.px(10.0))
                    .border_b_1()
                    .border_color(t.border)
                    .child(panels::title(self.panel))
                    .child(div().flex_1())
                    .children(actions)
                    .child(
                        div()
                            .id("float-back")
                            .debug_selector(|| "float-back".into())
                            .cursor_pointer()
                            .text_color(t.heat_2)
                            .on_click(cx.listener(|this, _, window, cx| {
                                if let Some(owner) = this.owner.upgrade() {
                                    owner.update(cx, |ws, cx| {
                                        ws.floating.remove(this.panel);
                                        cx.notify();
                                    });
                                }
                                window.remove_window();
                            }))
                            .child("Back to the dock"),
                    ),
            )
            .child(div().flex_1().min_h_0().min_w_0().child(self.content.clone()))
    }
}

impl Workspace {
    pub(super) fn float_panel(&mut self, panel: &'static str, _: &mut Window, cx: &mut Context<Self>) {
        if matches!(panel, "editor" | "terminal") {
            self.status_note("The editor and the terminal stay in the main window.", cx);
            return;
        }
        if self.focus_floating(panel, cx) {
            return;
        }
        if !dock::PANEL_IDS.contains(&panel) {
            return;
        }
        let content = self.panel_view(panel, cx);
        let owner = cx.entity().downgrade();
        let output = self.output.clone();
        let tests = self.tests.clone();
        let options = gpui_kit::WindowOptions {
            window_bounds: Some(gpui_kit::WindowBounds::Windowed(gpui_kit::Bounds::centered(
                None,
                gpui_kit::size(px(640.0), px(460.0)),
                cx,
            ))),
            titlebar: Some(gpui_kit::TitlebarOptions {
                title: Some(format!("{} · Forge", panels::title(panel)).into()),
                ..Default::default()
            }),
            window_min_size: Some(gpui_kit::size(px(240.0), px(180.0))),
            ..Default::default()
        };
        match gpui_kit::open_window(options, cx, |window, cx| {
            let view = cx.new(|_| FloatingPanel { panel, content, owner, output, tests });
            cx.new(|cx| gpui_kit::component::Root::new(view, window, cx))
        }) {
            Ok((handle, _)) => {
                self.floating.insert(panel, handle);
                cx.notify();
            }
            Err(error) => self.status_note(format!("Couldn't open a window for the panel: {error}"), cx),
        }
    }

    pub(super) fn focus_floating(&self, panel: &str, cx: &mut Context<Self>) -> bool {
        self.floating
            .get(panel)
            .is_some_and(|handle| handle.update(cx, |_, window, _| window.activate_window()).is_ok())
    }

    /// Exclude floating views from every task without changing saved layouts.
    pub(super) fn docked_root(&self) -> Option<DockNode> {
        let mut root = Some(self.tasks.root().clone());
        for panel in self.floating.keys() {
            root = root.and_then(|r| dock::remove_panel(&r, panel));
        }
        root
    }

    /// Map visible divider sizes back onto the saved split, reserving the
    /// shares of children whose panels are all in floating windows.
    pub(super) fn resize_docked(&mut self, split: &str, visible_sizes: &[f64]) {
        let Some(DockNode::Split { children, sizes, .. }) = dock::find_node(self.tasks.root(), split) else {
            return;
        };
        let visible: Vec<bool> = children
            .iter()
            .map(|child| dock::placed_panels(child).iter().any(|p| !self.floating.contains_key(p.as_str())))
            .collect();
        if visible.iter().filter(|v| **v).count() != visible_sizes.len() {
            return;
        }
        let reserved: f64 = sizes.iter().zip(&visible).filter(|(_, v)| !**v).map(|(s, _)| s).sum();
        let mut incoming = visible_sizes.iter();
        let next: Vec<f64> = sizes
            .iter()
            .zip(visible)
            .map(
                |(old, visible)| {
                    if visible {
                        incoming.next().copied().unwrap_or(*old) * (1.0 - reserved)
                    } else {
                        *old
                    }
                },
            )
            .collect();
        self.tasks.changed(dock::resize(self.tasks.root(), split, &next));
    }

    pub(super) fn panel_is_shown(&self, panel: &str) -> bool {
        self.floating.contains_key(panel) || self.tasks.is_shown(panel)
    }

    pub(super) fn close_floating(&mut self, cx: &mut gpui_kit::App) {
        for (_, handle) in self.floating.drain() {
            handle.update(cx, |_, window, _| window.remove_window()).ok();
        }
    }
}
