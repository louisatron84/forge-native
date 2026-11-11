//! Native OpenAPI endpoint picker, replacing `boxes/openApiDialog.ts`.
use crate::theme::ForgeTheme;
use forge_core::openapi::ApiSpec;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::{
    div, uniform_list, AppContext, Context, Entity, EventEmitter, InteractiveElement, IntoElement, ParentElement,
    Render, StatefulInteractiveElement, Styled, Window,
};
use std::{collections::HashSet, sync::Arc};
#[derive(Clone)]
pub enum OpenApiEvent {
    Load(String),
    Pick,
    Preview,
    Close,
}
pub struct OpenApiPanel {
    pub open: bool,
    pub status: String,
    pub spec: Option<Arc<ApiSpec>>,
    pub chosen: HashSet<String>,
    pub import_current: bool,
    pub serial: u64,
    pub source: Entity<InputState>,
    pub target: Entity<InputState>,
}
impl EventEmitter<OpenApiEvent> for OpenApiPanel {}
/// Stable content identity keeps an existing floating Box window in sync when
/// switching between manual boxes and the OpenAPI importer.
pub struct BoxContent {
    settings: Entity<super::connect::ConnectPanel>,
    openapi: Entity<OpenApiPanel>,
    _subscriptions: Vec<gpui_kit::Subscription>,
}
impl BoxContent {
    pub fn new(
        settings: Entity<super::connect::ConnectPanel>,
        openapi: Entity<OpenApiPanel>,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions =
            vec![cx.observe(&settings, |_, _, cx| cx.notify()), cx.observe(&openapi, |_, _, cx| cx.notify())];
        Self { settings, openapi, _subscriptions: subscriptions }
    }
}
impl Render for BoxContent {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.openapi.read(cx).open {
            self.openapi.clone().into_any_element()
        } else {
            self.settings.clone().into_any_element()
        }
    }
}
impl OpenApiPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            open: false,
            status: String::new(),
            spec: None,
            chosen: HashSet::new(),
            import_current: true,
            serial: 0,
            source: cx.new(|cx| InputState::new(window, cx).placeholder("OpenAPI address or JSON/YAML file")),
            target: cx.new(|cx| InputState::new(window, cx)),
        }
    }
    pub fn loaded(&mut self, spec: ApiSpec, window: &mut Window, cx: &mut Context<Self>) {
        self.chosen = spec.operations.iter().map(|o| o.id.clone()).collect();
        self.serial = self.serial.wrapping_add(1);
        self.target.update(cx, |t, cx| t.set_value(format!("api/{}_client.py", spec.slug), window, cx));
        self.status = spec.warnings.join("\n");
        self.spec = Some(Arc::new(spec));
        cx.notify();
    }
}
impl Render for OpenApiPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.global::<ForgeTheme>().0.clone();
        let mut view = div()
            .id("openapi-settings")
            .size_full()
            .overflow_y_scroll()
            .p(t.px(10.))
            .flex()
            .flex_col()
            .gap(t.px(8.))
            .text_color(t.fg)
            .text_size(t.px(12.))
            .child("Add API from OpenAPI / Swagger")
            .child(Input::new(&self.source).w_full())
            .child(
                div()
                    .flex()
                    .gap(t.px(10.))
                    .child(div().id("openapi-load").cursor_pointer().child("Load description").on_click(
                        cx.listener(|p, _, _, cx| cx.emit(OpenApiEvent::Load(p.source.read(cx).value().to_string()))),
                    ))
                    .child(
                        div()
                            .id("openapi-pick")
                            .cursor_pointer()
                            .child("Choose file…")
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(OpenApiEvent::Pick))),
                    ),
            )
            .child(div().text_color(t.fg_muted).child(self.status.clone()));
        if let Some(spec) = &self.spec {
            view = view
                .child(format!("{} {} · {} endpoints", spec.title, spec.version, spec.operations.len()))
                .child(spec.description.clone())
                .child(format!("Address: {}", spec.base_url))
                .child(format!("Environment names: {}", forge_core::openapi::env_keys(spec).join(", ")))
                .child("Client file")
                .child(Input::new(&self.target).w_full())
                .child(
                    div()
                        .id("openapi-import")
                        .cursor_pointer()
                        .child(if self.import_current {
                            "☑ Import selected functions into the current Python file"
                        } else {
                            "☐ Import selected functions into the current Python file"
                        })
                        .on_click(cx.listener(|p, _, _, cx| {
                            p.import_current = !p.import_current;
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .flex()
                        .gap(t.px(10.))
                        .child(div().id("openapi-all").cursor_pointer().child("All").on_click(cx.listener(
                            |p, _, _, cx| {
                                p.chosen =
                                    p.spec.iter().flat_map(|s| s.operations.iter().map(|o| o.id.clone())).collect();
                                cx.notify();
                            },
                        )))
                        .child(div().id("openapi-none").cursor_pointer().child("None").on_click(cx.listener(
                            |p, _, _, cx| {
                                p.chosen.clear();
                                cx.notify();
                            },
                        ))),
                );
            view = view.child(
                uniform_list(
                    "openapi-endpoints",
                    spec.operations.len(),
                    cx.processor(|p, range: std::ops::Range<usize>, _, cx| {
                        let t = cx.global::<ForgeTheme>().0.clone();
                        range
                            .filter_map(|i| {
                                let op = p.spec.as_ref()?.operations.get(i)?;
                                let id = op.id.clone();
                                Some(
                                    div()
                                        .id(("openapi-operation", i))
                                        .h(t.px(24.))
                                        .w_full()
                                        .text_ellipsis()
                                        .whitespace_nowrap()
                                        .cursor_pointer()
                                        .child(format!(
                                            "{} {} {} · {}",
                                            if p.chosen.contains(&op.id) { "☑" } else { "☐" },
                                            op.method,
                                            op.path,
                                            op.summary
                                        ))
                                        .on_click(cx.listener(move |p, _, _, cx| {
                                            if !p.chosen.remove(&id) {
                                                p.chosen.insert(id.clone());
                                            }
                                            cx.notify();
                                        })),
                                )
                            })
                            .collect()
                    }),
                )
                .h(t.px(200.))
                .w_full(),
            );
            view = view.child(
                div()
                    .id("openapi-preview")
                    .debug_selector(|| "openapi-preview".into())
                    .cursor_pointer()
                    .text_color(t.heat_2)
                    .child("Preview generated code")
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(OpenApiEvent::Preview))),
            );
        }
        view.child(
            div()
                .id("openapi-close")
                .cursor_pointer()
                .child("Back to API boxes")
                .on_click(cx.listener(|_, _, _, cx| cx.emit(OpenApiEvent::Close))),
        )
    }
}
