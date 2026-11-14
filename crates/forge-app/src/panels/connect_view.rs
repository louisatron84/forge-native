//! Request-first API settings with a persistent action footer. Keeps Forge's
//! native colours/density and progressively reveals shared and code settings.
use super::*;
use gpui_kit::{prelude::FluentBuilder, Div};
#[derive(Clone, Copy)]
enum Section {
    Api,
    Advanced,
    Code,
}
impl ConnectPanel {
    fn section(
        &self,
        id: &'static str,
        title: &'static str,
        summary: String,
        open: bool,
        section: Section,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Stateful<Div> {
        let t = cx.global::<ForgeTheme>().0.clone();
        div()
            .id(id)
            .debug_selector(move || id.into())
            .w_full()
            .cursor_pointer()
            .py(t.px(6.))
            .border_t_1()
            .border_color(t.border)
            .child(div().flex().justify_between().child(title).child(if open { "−" } else { "+" }))
            .when(!open && !summary.is_empty(), |d| {
                d.child(
                    div()
                        .text_color(t.fg_muted)
                        .text_size(t.px(10.))
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .child(summary),
                )
            })
            .on_click(cx.listener(move |p, _, _, cx| {
                match section {
                    Section::Api => p.show_api = !p.show_api,
                    Section::Advanced => p.show_advanced = !p.show_advanced,
                    Section::Code => p.show_code = !p.show_code,
                }
                cx.notify();
            }))
    }
    fn button(
        &self,
        id: &'static str,
        label: &'static str,
        event: ConnectEvent,
        primary: bool,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Stateful<Div> {
        let t = cx.global::<ForgeTheme>().0.clone();
        div()
            .id(id)
            .debug_selector(move || id.into())
            .h(t.px(30.))
            .px(t.px(10.))
            .flex()
            .items_center()
            .cursor_pointer()
            .rounded(t.px(3.))
            .border_1()
            .border_color(if primary { t.heat_2 } else { t.line_strong })
            .bg(if primary { t.heat_2 } else { t.raised })
            .text_color(if primary { t.on_heat } else { t.fg })
            .child(label)
            .on_click(cx.listener(move |p, _, _, cx| {
                if !p.testing || !matches!(event, ConnectEvent::TestCall) {
                    cx.emit(event.clone());
                }
            }))
    }
    pub(super) fn render_settings(&self, cx: &mut Context<Self>) -> gpui_kit::Stateful<Div> {
        let t = cx.global::<ForgeTheme>().0.clone();
        let base = div()
            .id("box-settings")
            .size_full()
            .flex()
            .flex_col()
            .min_h_0()
            .min_w_0()
            .bg(t.bg_panel)
            .text_color(t.fg)
            .text_size(t.px(12.));
        let Some(api) = self.current() else {
            return base.p(t.px(12.)).child(if self.has_project {
                "Add an API box from the Toolbox to get started."
            } else {
                "Open a project folder to connect an API."
            });
        };
        let Some(request) = api.requests.get(self.request) else { return base };
        let auth = match api.auth {
            BoxAuth::None => "No authentication",
            BoxAuth::Bearer { .. } => "Bearer token",
            BoxAuth::Header { .. } => "API key in header",
            BoxAuth::Query { .. } => "API key in query",
            BoxAuth::Basic { .. } => "Basic authentication",
        };
        let mut requests = div().flex().flex_wrap().gap(t.px(5.));
        for (i, r) in api.requests.iter().enumerate() {
            requests = requests.child(
                div()
                    .id(("api-request", i))
                    .px(t.px(7.))
                    .py(t.px(4.))
                    .rounded(t.px(3.))
                    .bg(if i == self.request { t.bg_active } else { t.bg_panel })
                    .text_color(if i == self.request { t.heat_2 } else { t.fg_muted })
                    .cursor_pointer()
                    .child(r.name.clone())
                    .on_click(cx.listener(move |p, _, window, cx| {
                        p.request = i;
                        p.refresh_form(window, cx);
                        cx.notify();
                    })),
            );
        }
        requests = requests.child(
            div()
                .id("connect-add-request")
                .debug_selector(|| "connect-add-request".into())
                .px(t.px(6.))
                .py(t.px(4.))
                .cursor_pointer()
                .text_color(t.heat_2)
                .child("+ Request")
                .on_click(cx.listener(|p, _, window, cx| {
                    if let Some(i) = p.selected {
                        let r = new_request(&p.boxes[i], &format!("request_{}", p.boxes[i].requests.len() + 1));
                        p.boxes[i].requests.push(r);
                        p.request = p.boxes[i].requests.len() - 1;
                        p.refresh_form(window, cx);
                        cx.emit(ConnectEvent::Changed);
                        cx.notify();
                    }
                })),
        );
        let header = div()
            .flex_none()
            .p(t.px(10.))
            .border_b_1()
            .border_color(t.border)
            .flex()
            .flex_col()
            .gap(t.px(8.))
            .child(self.render_field(Field::Name, "name", "API name", cx))
            .child(requests);
        let mut body = div()
            .id("connect-form")
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_y_scroll()
            .p(t.px(10.))
            .flex()
            .flex_col()
            .gap(t.px(9.))
            .child(self.render_field(Field::Request, "request", "Request name", cx))
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap(t.px(6.))
                    .child(
                        div()
                            .id("connect-method")
                            .cursor_pointer()
                            .flex_none()
                            .h(t.px(36.))
                            .px(t.px(8.))
                            .flex()
                            .items_center()
                            .border_1()
                            .border_color(t.line_strong)
                            .rounded(t.px(3.))
                            .text_color(t.heat_2)
                            .child(request.method.clone())
                            .child(" ▾")
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(ConnectEvent::PickMethod))),
                    )
                    .child(div().flex_1().min_w_0().child(self.render_field(Field::Url, "url", "URL or path", cx))),
            );
        if request.url.contains('{') || !request.test_args.is_empty() {
            body = body.child(self.render_field(Field::Args, "args", "Test values (name=value)", cx));
        }
        body = body.child(self.render_field(Field::Output, "output", "Save response to CSV", cx));
        let status_color = request.test.as_ref().map_or(t.fg_muted, |test| if test.ok { t.ok } else { t.err });
        body = body.child(
            div()
                .id("connect-test-action")
                .flex()
                .items_center()
                .gap(t.px(8.))
                .child(self.button(
                    "connect-test-call",
                    if self.testing { "Calling…" } else { "⚡ Test call" },
                    ConnectEvent::TestCall,
                    true,
                    cx,
                ))
                .child(
                    div().id("connect-status").text_size(t.px(10.)).text_color(status_color).child(self.status.clone()),
                ),
        );
        if request.test.as_ref().is_some_and(|test| test.ok && test.sample.is_some()) {
            body = body.child(div().text_color(t.heat_2).child("Response fields")).child(self.response_view(cx));
        }
        body = body.child(self.section(
            "connect-api-settings",
            "API settings",
            auth.into(),
            self.show_api,
            Section::Api,
            cx,
        ));
        if self.show_api {
            body = body.child(self.render_field(Field::Base, "base", "Base URL (optional)", cx)).child(
                div()
                    .id("connect-auth")
                    .debug_selector(|| "connect-auth".into())
                    .cursor_pointer()
                    .text_color(t.heat_2)
                    .child(auth)
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(ConnectEvent::PickAuth))),
            );
            let fields: &[(Field, &str, &str)] = match api.auth {
                BoxAuth::None => &[],
                BoxAuth::Bearer { .. } => &[(Field::Env, "env", "Token environment name")],
                BoxAuth::Header { .. } | BoxAuth::Query { .. } => &[
                    (Field::AuthName, "auth-name", "Header / query name"),
                    (Field::Env, "env", "Key environment name"),
                ],
                BoxAuth::Basic { .. } => &[
                    (Field::UserEnv, "user-env", "Username environment name"),
                    (Field::PassEnv, "pass-env", "Password environment name"),
                ],
            };
            for (field, id, label) in fields {
                body = body.child(self.render_field(*field, id, label, cx));
            }
            body = body
                .child(
                    div()
                        .text_size(t.px(10.))
                        .text_color(t.fg_muted)
                        .child("Credentials stay in .env; generated code reads their environment names."),
                )
                .child(
                    div()
                        .id("connect-env")
                        .cursor_pointer()
                        .text_color(t.link)
                        .child("Edit .env")
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(ConnectEvent::Env))),
                );
            if self.env_ignored {
                body = body.child(div().text_color(t.fg_muted).text_size(t.px(10.)).child(".env is ignored by Git"));
            } else {
                body = body.child(
                    div()
                        .id("connect-ignore-env")
                        .debug_selector(|| "connect-ignore-env".into())
                        .cursor_pointer()
                        .text_color(t.link)
                        .child("Add .env to .gitignore")
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(ConnectEvent::IgnoreEnv))),
                );
            }
        }
        let advanced = format!("{} query parameters, {} headers", request.query.len(), request.headers.len());
        body = body.child(self.section(
            "connect-advanced",
            "Request options",
            advanced,
            self.show_advanced,
            Section::Advanced,
            cx,
        ));
        if self.show_advanced {
            body = body
                .child(self.render_field(Field::Query, "query", "Query (name=value; ?name makes an argument)", cx))
                .child(self.render_field(Field::Headers, "headers", "Headers (name=value)", cx));
            if matches!(request.method.as_str(), "POST" | "PUT" | "PATCH") || !request.body.is_empty() {
                body = body.child(self.render_field(Field::Body, "body", "JSON body", cx));
            }
        }
        body = body.child(self.section(
            "connect-code-options",
            "Python output",
            api.target.file.clone(),
            self.show_code,
            Section::Code,
            cx,
        ));
        if self.show_code {
            body = body
                .child(self.render_field(Field::Target, "target", "Python file", cx))
                .child(
                    div()
                        .id("connect-target-mode")
                        .cursor_pointer()
                        .text_color(t.link)
                        .child(if api.target.mode == "new" { "New file" } else { "Append to existing file" })
                        .on_click(cx.listener(|p, _, window, cx| {
                            if let Some(i) = p.selected {
                                p.boxes[i].target.mode =
                                    if p.boxes[i].target.mode == "new" { "append" } else { "new" }.into();
                                p.refresh_form(window, cx);
                                cx.emit(ConnectEvent::Changed);
                            }
                        })),
                )
                .child(self.render_field(Field::Function, "function", "Function name (optional)", cx));
            if api.detached {
                body = body.child(
                    div()
                        .id("connect-code-updates")
                        .cursor_pointer()
                        .text_color(t.link)
                        .child("Re-enable automatic code updates")
                        .on_click(cx.listener(|p, _, _, cx| {
                            if let Some(i) = p.selected {
                                p.boxes[i].detached = false;
                                cx.emit(ConnectEvent::Changed);
                                cx.notify();
                            }
                        })),
                );
            }
            if api.requests.len() > 1 {
                body = body.child(
                    div()
                        .id("connect-remove-request")
                        .cursor_pointer()
                        .text_color(t.err)
                        .child("Remove this request…")
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(ConnectEvent::RemoveRequest))),
                );
            }
            body = body.child(
                div()
                    .id("connect-remove")
                    .cursor_pointer()
                    .text_color(t.err)
                    .child("Remove API box…")
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(ConnectEvent::Remove))),
            );
        }
        let footer =
            div().flex_none().p(t.px(10.)).border_t_1().border_color(t.border).flex().flex_col().gap(t.px(7.)).child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(t.px(7.))
                    .child(self.button("connect-preview", "Review code", ConnectEvent::Preview, false, cx))
                    .children(
                        request
                            .test
                            .as_ref()
                            .filter(|test| test.ok && test.sample.is_some() && !request.ticked.is_empty())
                            .map(|_| self.button("connect-open-csv", "Open CSV", ConnectEvent::OpenCsv, false, cx)),
                    ),
            );
        if request.test.as_ref().is_some_and(|test| test.ok && test.sample.is_some()) {
            let settings = div()
                .id("connect-test-settings")
                .debug_selector(|| "connect-test-settings".into())
                .w(gpui_kit::relative(0.42))
                .min_w(t.px(360.))
                .min_h_0()
                .flex_none()
                .flex()
                .flex_col()
                .child(header)
                .child(body)
                .child(footer);
            base.child(div().flex().flex_1().min_h_0().min_w_0().child(settings).child(self.test_output_view(cx)))
        } else {
            base.child(header).child(body).child(footer)
        }
    }
}
