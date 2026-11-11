//! Drawing the dock: splits with draggable dividers and tab stacks of panels.
//! In Edit Layout mode tabs can be dragged onto a side or the middle of a
//! stack, closed, and added back. Replaces `src/renderer/dock/dockView.ts`.

use super::Workspace;
use crate::app_state;
use crate::palette::{corner_mark, Choice};
use crate::panels;
use crate::theme::{ForgeTheme, Theme};
use forge_core::dock::{self as model, Dir, DockNode, Zone};
use gpui_kit::{
    div, prelude::FluentBuilder, px, relative, Animation, AnimationExt, AnyElement, AppContext, Bounds, Context,
    DragMoveEvent, InteractiveElement, IntoElement, ParentElement, Pixels, Point, Render, SharedString,
    StatefulInteractiveElement, Styled, Window,
};

/// A divider being dragged: which split, and which pair of its children.
#[derive(Clone)]
pub(crate) struct DraggedDivider {
    split: String,
    index: usize,
}

/// A panel tab being dragged (Edit Layout mode).
#[derive(Clone)]
pub(crate) struct DraggedPanel {
    panel: String,
}

/// What's drawn under the mouse while dragging.
pub(crate) struct DragGhost(Option<SharedString>);

#[derive(Default)]
pub(super) struct DockMotion {
    seen: Option<model::Transition>,
    active: Option<model::Transition>,
}

impl DockMotion {
    fn observe(&mut self, transition: Option<model::Transition>, enabled: bool) -> Option<model::Transition> {
        if transition != self.seen {
            self.seen = transition;
            self.active = transition.filter(|_| enabled);
        }
        if !enabled {
            self.active = None;
        }
        self.active
    }
}

impl Render for DragGhost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.global::<ForgeTheme>().0.clone();
        match &self.0 {
            None => div(),
            Some(title) => div()
                .px(px(9.0))
                .py(px(4.0))
                .bg(t.raised)
                .border_1()
                .border_color(t.heat_2)
                .text_size(px(10.0))
                .text_color(t.heat_2)
                .child(title.to_uppercase()),
        }
    }
}

impl Workspace {
    pub(super) fn render_dock(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let root = self.docked_root();
        let t = cx.global::<ForgeTheme>().0.clone();
        let dock = div()
            .debug_selector(|| "dock-body".into())
            .size_full()
            .flex()
            .min_w_0()
            .min_h_0()
            .when(self.editing_layout, |d| d.bg(t.bg))
            .children(root.as_ref().map(|root| self.render_node(root, window, cx)));
        match self.dock_motion.observe(self.tasks.transition(), app_state::settings(cx).animations) {
            Some(transition) => {
                let task = transition.kind == model::TransitionKind::Task;
                dock.with_animation(
                    SharedString::from(format!("dock-motion-{}", transition.revision)),
                    Animation::new(std::time::Duration::from_millis(if task { 220 } else { 180 }))
                        .with_easing(|progress| 1.0 - (1.0 - progress).powi(3)),
                    move |dock, progress| {
                        dock.relative().top(px(if task { 6.0 * (1.0 - progress) } else { 0.0 })).opacity(if task {
                            progress
                        } else {
                            0.4 + 0.6 * progress
                        })
                    },
                )
                .into_any_element()
            }
            None => dock.into_any_element(),
        }
    }

    fn render_node(&mut self, node: &DockNode, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        match node {
            DockNode::Stack { id, panels, active, collapsed } => {
                self.render_stack(id, panels, active, *collapsed, window, cx)
            }
            DockNode::Split { id, dir, children, sizes } => self.render_split(id, *dir, children, sizes, window, cx),
        }
    }

    fn render_split(
        &mut self,
        id: &str,
        dir: Dir,
        children: &[DockNode],
        sizes: &[f64],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = cx.global::<ForgeTheme>().0.clone();
        let row = dir == Dir::Row;
        let mut el = div().flex().size_full().min_w_0().min_h_0();
        el = if row { el.flex_row() } else { el.flex_col() };
        for (i, child) in children.iter().enumerate() {
            if i > 0 {
                el = el.child(self.divider(id, i - 1, row, &t));
            }
            let collapsed = matches!(child, DockNode::Stack { collapsed: true, .. });
            let mut cell = div().flex().min_w_0().min_h_0().overflow_hidden();
            if collapsed {
                // A collapsed stack (the drawer after Ctrl+J) is only its tab strip.
                cell = cell.flex_none();
            } else {
                cell = cell.flex_basis(relative(0.0));
                let s = cell.style();
                s.flex_grow = Some((sizes.get(i).copied().unwrap_or(0.0).max(0.0) * 1000.0) as f32);
                s.flex_shrink = Some(1.0);
            }
            el = el.child(cell.child(self.render_node(child, window, cx)));
        }
        let split = id.to_owned();
        el.on_drag_move(cx.listener(move |this, e: &DragMoveEvent<DraggedDivider>, _window, cx| {
            let d = e.drag(cx).clone();
            if d.split == split {
                this.resize_from_drag(&split, d.index, row, e.bounds, e.event.position, cx);
            }
        }))
        .into_any_element()
    }

    fn divider(&self, split: &str, index: usize, row: bool, t: &Theme) -> impl IntoElement {
        let width = px(if self.editing_layout { 8.0 } else { 4.0 });
        div()
            .id(SharedString::from(format!("divider-{split}-{index}")))
            .flex_none()
            .bg(if self.editing_layout { t.line_strong } else { t.border })
            .hover(|s| s.bg(t.heat_3))
            .when(row, |d| d.w(width).h_full().cursor_col_resize())
            .when(!row, |d| d.h(width).w_full().cursor_row_resize())
            .on_drag(DraggedDivider { split: split.to_owned(), index }, |_, _, _, cx| {
                cx.stop_propagation();
                cx.new(|_| DragGhost(None))
            })
    }

    /// Move the boundary between two children of a split to the mouse.
    fn resize_from_drag(
        &mut self,
        split: &str,
        index: usize,
        row: bool,
        bounds: Bounds<Pixels>,
        pos: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(docked) = self.docked_root() else { return };
        let Some(DockNode::Split { sizes, .. }) = model::find_node(&docked, split) else { return };
        if index + 1 >= sizes.len() {
            return;
        }
        let (start, len, at) = if row {
            (bounds.origin.x, bounds.size.width, pos.x)
        } else {
            (bounds.origin.y, bounds.size.height, pos.y)
        };
        let len = f32::from(len);
        if len <= 0.0 {
            return;
        }
        let frac = (f32::from(at - start) / len) as f64;
        let before: f64 = sizes[..index].iter().sum();
        let pair = sizes[index] + sizes[index + 1];
        let a = (frac - before).clamp(0.0, pair);
        let mut next = sizes.clone();
        next[index] = a;
        next[index + 1] = pair - a;
        if next != *sizes {
            self.resize_docked(split, &next);
            self.layouts_changed(cx);
            cx.notify();
        }
    }

    fn render_stack(
        &mut self,
        id: &str,
        panels: &[String],
        active: &str,
        collapsed: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = cx.global::<ForgeTheme>().0.clone();
        let editing = self.editing_layout;
        let tabs = panels.iter().map(|p| {
            let on = p == active;
            let title = panels::title(p);
            let (p1, p2, p3) = (p.clone(), p.clone(), p.clone());
            let stack = id.to_owned();
            div()
                .id(SharedString::from(format!("dock-tab-{p}")))
                .flex()
                .items_center()
                .gap(t.px(6.0))
                .h(t.px(24.0))
                .px(t.px(9.0))
                .rounded(t.px(2.0))
                .text_size(t.px(10.0))
                .whitespace_nowrap()
                .cursor_pointer()
                .text_color(if on { t.heat_2 } else { t.fg_muted })
                .when(on, |d| d.border_b_2().border_color(t.heat_2))
                .when(!on, |d| d.hover(|s| s.text_color(t.fg).bg(t.bg_hover)))
                .on_click(cx.listener(move |this, _, window, cx| this.click_dock_tab(&stack, &p1, window, cx)))
                .child(title.to_uppercase())
                .when(editing && p != "editor", |d| {
                    d.child(
                        div()
                            .id(SharedString::from(format!("dock-close-{p}")))
                            .px(t.px(2.0))
                            .text_color(t.fg_muted)
                            .hover(|s| s.text_color(t.err))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.tasks.close_panel(&p2);
                                this.layouts_changed(cx);
                            }))
                            .child("✕"),
                    )
                })
                .when(editing, |d| {
                    d.on_drag(DraggedPanel { panel: p3 }, move |_, _, _, cx| cx.new(|_| DragGhost(Some(title.into()))))
                })
        });
        let strip = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(t.px(6.0))
            .h(t.px(30.0))
            .pl(t.px(10.0))
            .pr(t.px(6.0))
            .border_b_1()
            .border_color(t.border)
            .child(div().flex().gap(t.px(2.0)).min_w_0().overflow_hidden().children(tabs))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .justify_end()
                    .min_w_0()
                    .when(!collapsed && !matches!(active, "editor" | "terminal"), |d| {
                        d.child(
                            div()
                                .id(SharedString::from(format!("float-{active}")))
                                .debug_selector({
                                    let name = format!("float-{active}");
                                    move || name.clone()
                                })
                                .px(t.px(6.0))
                                .cursor_pointer()
                                .text_color(t.fg_muted)
                                .hover(|s| s.text_color(t.heat_2))
                                .tooltip(|window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new("Open panel in a separate window")
                                        .build(window, cx)
                                })
                                .on_click({
                                    let panel = model_panel_id(active);
                                    cx.listener(move |this, _, window, cx| {
                                        if let Some(panel) = panel {
                                            this.float_panel(panel, window, cx);
                                        }
                                    })
                                })
                                .child("↗"),
                        )
                    })
                    .when(active == "terminal" && !collapsed, |d| {
                        d.child(crate::panels::terminal::actions(&self.terminal, &t))
                    })
                    .when(active == "output" && !collapsed, |d| {
                        d.child(crate::panels::output::actions(&self.output, &t))
                    })
                    .when(active == "tests" && !collapsed, |d| d.child(crate::panels::tests::actions(&self.tests, &t))),
            );
        let body = (!collapsed).then(|| {
            let content = if active == "editor" {
                self.render_editor_area(window, cx)
            } else {
                match panels.iter().find(|p| p.as_str() == active).and_then(|p| model_panel_id(p)) {
                    Some(pid) => self.panel_view(pid, cx).into_any_element(),
                    None => div().into_any_element(),
                }
            };
            div().relative().flex_1().min_h_0().min_w_0().flex().flex_col().child(content)
        });
        let sid = id.to_owned();
        let sid2 = id.to_owned();
        let marker = self.drop_target.as_ref().filter(|(s, _)| s == id).map(|(_, z)| drop_marker(*z, &t));
        div()
            .id(SharedString::from(format!("dock-stack-{id}")))
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .min_h_0()
            .bg(t.bg_panel)
            .when(editing, |d| d.border_1().border_color(t.line_strong))
            .child(corner_mark(t.heat_3))
            .child(strip)
            .children(body)
            .children(marker)
            .when(editing, |d| {
                d.on_drag_move(cx.listener(move |this, e: &DragMoveEvent<DraggedPanel>, _, cx| {
                    let b = e.bounds;
                    let p = e.event.position;
                    if !b.contains(&p) {
                        if this.drop_target.as_ref().is_some_and(|(s, _)| *s == sid) {
                            this.drop_target = None;
                            cx.notify();
                        }
                        return;
                    }
                    let zone = model::zone_at(
                        f32::from(b.origin.x) as f64,
                        f32::from(b.origin.y) as f64,
                        f32::from(b.size.width) as f64,
                        f32::from(b.size.height) as f64,
                        f32::from(p.x) as f64,
                        f32::from(p.y) as f64,
                    );
                    if this.drop_target.as_ref() != Some(&(sid.clone(), zone)) {
                        this.drop_target = Some((sid.clone(), zone));
                        cx.notify();
                    }
                }))
                .on_drop(cx.listener(move |this, d: &DraggedPanel, _, cx| {
                    let zone =
                        this.drop_target.take().filter(|(s, _)| *s == sid2).map(|(_, z)| z).unwrap_or(Zone::Center);
                    this.tasks.drop_panel(&d.panel, &sid2, zone);
                    this.layouts_changed(cx);
                    cx.notify();
                }))
            })
            .into_any_element()
    }

    fn click_dock_tab(&mut self, stack: &str, panel: &str, window: &mut Window, cx: &mut Context<Self>) {
        let mut root = model::activate(self.tasks.root(), panel);
        // Clicking a tab of a collapsed stack (the drawer) opens it again.
        if matches!(model::find_node(&root, stack), Some(DockNode::Stack { collapsed: true, .. })) {
            root = model::set_collapsed(&root, stack, false);
        }
        if &root != self.tasks.root() {
            self.tasks.changed(root);
            self.layouts_changed(cx);
        }
        if panel == "editor" {
            self.focus_editor(window, cx);
        }
        cx.notify();
    }

    /// The floating bar of Edit Layout mode: add a panel back, reset, done.
    pub(super) fn render_edit_bar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.global::<ForgeTheme>().0.clone();
        let hidden: Vec<_> =
            self.tasks.hidden_panels().into_iter().filter(|p| !self.floating.contains_key(p)).collect();
        let button = |id: &'static str, label: &'static str, primary: bool| {
            div()
                .id(id)
                .px(t.px(8.0))
                .py(t.px(3.0))
                .border_1()
                .border_color(if primary { t.heat_2 } else { t.border })
                .rounded(t.px(2.0))
                .bg(if primary { t.heat_3 } else { t.bg })
                .text_color(if primary { t.on_heat } else { t.fg })
                .text_size(t.px(12.0))
                .cursor_pointer()
                .child(label)
        };
        div().absolute().bottom(t.px(36.0)).left_0().right_0().flex().justify_center().child(
            div()
                .flex()
                .items_center()
                .gap(t.px(10.0))
                .px(t.px(12.0))
                .py(t.px(8.0))
                .bg(t.raised)
                .border_1()
                .border_color(t.heat_2)
                .shadow_md()
                .child(
                    div()
                        .text_color(t.fg_muted)
                        .text_size(t.px(12.0))
                        .child("Edit Layout: drag tabs onto a side or the middle of a panel"),
                )
                .child(button("edit-add", "Add panel…", false).when(hidden.is_empty(), |d| d.opacity(0.5)).on_click(
                    cx.listener(move |this, _, window, cx| {
                        let choices = this
                            .tasks
                            .hidden_panels()
                            .into_iter()
                            .filter(|p| !this.floating.contains_key(p))
                            .map(|p| {
                                Choice::new(panels::title(p), move |ws, _, cx| {
                                    ws.tasks.add_panel(p);
                                    ws.layouts_changed(cx);
                                })
                            })
                            .collect();
                        this.choose("Add a panel to this task", choices, window, cx);
                    }),
                ))
                .child(button("edit-reset", "Reset this task", false).on_click(cx.listener(|this, _, _, cx| {
                    this.tasks.reset_current();
                    this.layouts_changed(cx);
                })))
                .child(
                    button("edit-done", "Done", true)
                        .on_click(cx.listener(|this, _, _, cx| this.set_editing_layout(false, cx))),
                ),
        )
    }
}

/// The dock's panel ids are `&'static str`: find the one with this name.
fn model_panel_id(p: &str) -> Option<&'static str> {
    model::PANEL_IDS.iter().copied().find(|id| *id == p)
}

/// Where a dragged panel would land on a stack.
fn drop_marker(zone: Zone, t: &Theme) -> impl IntoElement {
    let m = div().absolute().bg(t.match_bg).border_2().border_color(t.heat_3);
    match zone {
        Zone::Center => m.top(relative(0.3)).bottom(relative(0.3)).left(relative(0.3)).right(relative(0.3)),
        Zone::Left => m.left_0().top_0().bottom_0().w(relative(0.5)),
        Zone::Right => m.right_0().top_0().bottom_0().w(relative(0.5)),
        Zone::Top => m.left_0().right_0().top_0().h(relative(0.5)),
        Zone::Bottom => m.left_0().right_0().bottom_0().h(relative(0.5)),
    }
}
