//! API box settings arranged in the same order as the Electron form.
use super::*;
use gpui_kit::Div;
impl ConnectPanel {
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
            .h(t.px(28.))
            .flex_none()
            .px(t.px(10.))
            .flex()
            .items_center()
            .cursor_pointer()
            .border_1()
            .border_color(t.line_strong)
            .bg(if primary { t.fg } else { t.bg_panel })
            .text_color(if primary { t.bg_panel } else { t.fg })
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
            BoxAuth::None => "None",
            BoxAuth::Bearer { .. } => "Bearer token",
            BoxAuth::Header { .. } => "API key in header",
            BoxAuth::Query { .. } => "API key in query",
            BoxAuth::Basic { .. } => "Basic authentication",
        };
        let mut requests = div().flex().flex_wrap().gap(t.px(5.)).pt(t.px(6.)).border_t_1().border_color(t.border);
        for (i, r) in api.requests.iter().enumerate() {
            requests = requests.child(
                div()
                    .id(("api-request", i))
                    .px(t.px(7.))
                    .py(t.px(4.))
                    .border_1()
                    .border_color(if i == self.request { t.line_strong } else { t.bg_panel })
                    .bg(if i == self.request { t.raised } else { t.bg_panel })
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
        let mut shared = div()
            .flex()
            .flex_col()
            .gap(t.px(6.))
            .border_t_1()
            .border_color(t.border)
            .pt(t.px(7.))
            .child(div().font_family(crate::theme::CODE_FONT).child("BOX"));

        shared = shared.child(self.render_field(Field::Base, "base", "Base URL", cx)).child(
            div().flex().items_center().gap(t.px(8.)).child(div().w(t.px(102.)).flex_none().child("Auth")).child(
                div()
                    .id("connect-auth")
                    .debug_selector(|| "connect-auth".into())
                    .cursor_pointer()
                    .text_color(t.heat_2)
                    .px(t.px(8.))
                    .py(t.px(3.))
                    .border_1()
                    .border_color(t.line_strong)
                    .child(format!("{auth} ?"))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(ConnectEvent::PickAuth))),
            ),
        );
        let fields: &[(Field, &str, &str)] = match api.auth {
            BoxAuth::None => &[],
            BoxAuth::Bearer { .. } => &[(Field::Env, "env", "Token environment name")],
            BoxAuth::Header { .. } | BoxAuth::Query { .. } => {
                &[(Field::AuthName, "auth-name", "Header / query name"), (Field::Env, "env", "Key environment name")]
            }
            BoxAuth::Basic { .. } => &[
                (Field::UserEnv, "user-env", "Username environment name"),
                (Field::PassEnv, "pass-env", "Password environment name"),
            ],
        };
        for (field, id, label) in fields {
            shared = shared.child(self.render_field(*field, id, label, cx));
        }
        if !fields.is_empty() {
            shared = shared
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
                shared =
                    shared.child(div().text_color(t.fg_muted).text_size(t.px(10.)).child(".env is ignored by Git"));
            } else {
                shared = shared.child(
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
        shared = shared.child(
            div()
                .flex()
                .items_center()
                .gap(t.px(8.))
                .child(div().flex_1().min_w_0().child(self.render_field(Field::Target, "target", "Code goes in", cx)))
                .child(
                    div()
                        .id("connect-target-mode")
                        .cursor_pointer()
                        .px(t.px(8.))
                        .py(t.px(3.))
                        .border_1()
                        .border_color(t.line_strong)
                        .child(if api.target.mode == "new" { "a new file ?" } else { "append to file ?" })
                        .on_click(cx.listener(|p, _, window, cx| {
                            if let Some(i) = p.selected {
                                p.boxes[i].target.mode =
                                    if p.boxes[i].target.mode == "new" { "append" } else { "new" }.into();
                                p.refresh_form(window, cx);
                                cx.emit(ConnectEvent::Changed);
                                cx.notify();
                            }
                        })),
                ),
        );
        if api.detached {
            shared = shared.child(
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
            shared = shared.child(
                div()
                    .id("connect-remove-request")
                    .cursor_pointer()
                    .text_color(t.err)
                    .child("Remove this request…")
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(ConnectEvent::RemoveRequest))),
            );
        }
        let header = div()
            .flex_none()
            .p(t.px(10.))
            .border_b_1()
            .border_color(t.border)
            .flex()
            .flex_col()
            .gap(t.px(8.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(t.px(10.))
                    .child(div().flex_1().min_w_0().child(self.render_field(Field::Name, "name", "", cx)))
                    .child(self.button("connect-remove", "Remove box", ConnectEvent::Remove, false, cx)),
            )
            .child(shared)
            .child(requests);
        let mut body = div()
            .id("connect-form")
            .flex_none()
            .min_w_0()
            .p(t.px(10.))
            .flex()
            .flex_col()
            .gap(t.px(9.))
            .child(self.render_field(Field::Request, "request", "Name", cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .child(div().w(t.px(102.)).flex_none().child("Request"))
                    .gap(t.px(6.))
                    .child(
                        div()
                            .id("connect-method")
                            .cursor_pointer()
                            .flex_none()
                            .h(t.px(28.))
                            .px(t.px(8.))
                            .flex()
                            .items_center()
                            .border_1()
                            .border_color(t.line_strong)
                            .text_color(t.heat_2)
                            .child(request.method.clone())
                            .child(" ▾")
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(ConnectEvent::PickMethod))),
                    )
                    .child(div().flex_1().min_w_0().child(self.render_field(Field::Url, "url", "", cx))),
            );
        if request.url.contains('{') || !request.test_args.is_empty() {
            body = body.child(self.render_field(Field::Args, "args", "Test values (name=value)", cx));
        }

        for (field, id, label, add, open) in [
            (Field::Query, "query", "Query parameters", "+ Parameter", self.show_query || !request.query.is_empty()),
            (Field::Headers, "headers", "Headers", "+ Header", self.show_headers || !request.headers.is_empty()),
        ] {
            body = body.child(div().child(label));
            if open {
                let hint = if field == Field::Query {
                    "name=value (prefix a name with ? to make it an argument)"
                } else {
                    "name=value"
                };
                body = body.child(self.render_field(field, id, hint, cx));
            } else {
                body = body.child(
                    div()
                        .id(format!("connect-edit-{id}"))
                        .text_center()
                        .cursor_pointer()
                        .py(t.px(3.))
                        .child(add)
                        .on_click(cx.listener(move |p, _, window, cx| {
                            if field == Field::Query {
                                p.show_query = true;
                            } else {
                                p.show_headers = true;
                            }
                            if let FieldInput::Many(input) = &p.inputs[&field] {
                                use gpui_kit::Focusable;
                                window.focus(&input.read(cx).focus_handle(cx), cx);
                            }
                            cx.notify();
                        })),
                );
            }
        }
        if matches!(request.method.as_str(), "POST" | "PUT" | "PATCH") || !request.body.is_empty() {
            body = body.child(self.render_field(Field::Body, "body", "JSON body", cx));
        }
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
        body = body.child(
            div()
                .text_color(t.fg_muted)
                .child(format!("A test call saves the rows it gets to {} in your project.", request.output)),
        );
        if request.test.as_ref().is_some_and(|test| test.ok && test.sample.is_some()) {
            body = body.child(div().text_color(t.heat_2).child("Fields to save")).child(self.response_view(cx));
        }
        if self.response_preview.is_some() {
            body = body.child(self.test_output_view(cx));
        }
        let open_csv =
            request.test.as_ref().is_some_and(|test| test.ok && test.sample.is_some() && !request.ticked.is_empty());
        body = body.child(
            div()
                .flex()
                .items_center()
                .gap(t.px(6.))
                .child(div().flex_1().min_w_0().child(self.render_field(Field::Output, "output", "Save to (CSV)", cx)))
                .children(
                    open_csv.then(|| self.button("connect-open-csv", "Open CSV", ConnectEvent::OpenCsv, false, cx)),
                ),
        );
        body = body.child(self.render_field(Field::Function, "function", "Function", cx));
        let footer =
            div().flex_none().p(t.px(10.)).border_t_1().border_color(t.border).flex().flex_col().gap(t.px(7.)).child(
                div().flex().flex_wrap().gap(t.px(7.)).child(self.button(
                    "connect-preview",
                    "Review code",
                    ConnectEvent::Preview,
                    false,
                    cx,
                )),
            );
        let footer =
            footer.child(div().text_color(t.fg_muted).child(format!("Code in {} follows this box.", api.target.file)));
        base.child(
            div()
                .id("connect-test-settings")
                .debug_selector(|| "connect-test-settings".into())
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .child(header)
                .child(body),
        )
        .child(footer)
    }
}
