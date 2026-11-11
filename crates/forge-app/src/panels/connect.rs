//! Native Toolbox and API-box settings, replacing `boxes/toolbox.ts` and
//! `boxes/boxSettings.ts`. Metadata and secret variable names, never secrets.
use crate::theme::ForgeTheme;
use forge_core::{
    box_code::auth_env,
    boxes::{new_api_box, new_request, rename_box, ApiBox, BoxAuth, KeyValue},
};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::{
    div, AnyElement, App, AppContext, Context, Entity, EventEmitter, InteractiveElement, IntoElement, ParentElement,
    Render, StatefulInteractiveElement, Styled, Subscription, Window,
};
use std::collections::HashMap;
#[path = "connect_view.rs"]
mod view;

#[derive(Clone)]
pub enum ConnectEvent {
    OpenApi,
    Add,
    Selected,
    Changed,
    Preview,
    Apply,
    TestCall,
    Env,
    PickMethod,
    PickAuth,
    Remove,
    RemoveRequest,
    OpenCsv,
    IgnoreEnv,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Field {
    Name,
    Base,
    Target,
    Request,
    Url,
    Output,
    Body,
    Query,
    Headers,
    Args,
    Env,
    AuthName,
    UserEnv,
    PassEnv,
    Function,
}
const FIELDS: [(Field, &str, &str, bool); 15] = [
    (Field::Name, "name", "API name", false),
    (Field::Base, "base", "Base URL", false),
    (Field::Target, "target", "Python file", false),
    (Field::Request, "request", "Request name", false),
    (Field::Url, "url", "URL or path", false),
    (Field::Output, "output", "CSV file", false),
    (Field::Function, "function", "Python function name (optional)", false),
    (Field::Body, "body", "JSON body", true),
    (Field::Query, "query", "Query (name=value; ?name makes an argument)", true),
    (Field::Headers, "headers", "Headers (name=value)", true),
    (Field::Args, "args", "Test arguments (name=value)", true),
    (Field::Env, "env", "Secret environment variable", false),
    (Field::AuthName, "auth-name", "Auth header / query name", false),
    (Field::UserEnv, "user-env", "Username environment variable", false),
    (Field::PassEnv, "pass-env", "Password environment variable", false),
];
enum FieldInput {
    Line(Entity<InputState>),
    Many(Entity<TextareaState>),
}
pub struct ConnectPanel {
    pub boxes: Vec<ApiBox>,
    pub selected: Option<usize>,
    pub request: usize,
    pub has_project: bool,
    pub status: String,
    show_api: bool,
    show_advanced: bool,
    show_code: bool,
    pub testing: bool,
    pub env_ignored: bool,
    response_preview: String,
    inputs: HashMap<Field, FieldInput>,
    _subscriptions: Vec<Subscription>,
}
impl EventEmitter<ConnectEvent> for ConnectPanel {}
impl ConnectPanel {
    pub fn test_result(
        &mut self,
        box_id: &str,
        request_id: &str,
        result: forge_sys::connect::TestCallResult,
        cx: &mut Context<Self>,
    ) {
        self.testing = false;
        use forge_core::{
            api_fields::{default_ticks, field_tree, pick_records},
            boxes::TestSummary,
        };
        let Some(request) = self
            .boxes
            .iter_mut()
            .find(|b| b.id == box_id)
            .and_then(|b| b.requests.iter_mut().find(|r| r.id == request_id))
        else {
            return;
        };
        let mut summary = TestSummary {
            ok: result.ok,
            at: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis()
                as f64,
            status: result.status.map(f64::from),
            status_text: result.status_text,
            ms: Some(result.ms),
            error: result.error,
            ..Default::default()
        };
        if result.ok {
            if let Some(json) = result.json {
                let (records, path) = pick_records(&json, request.records_path.as_deref());
                let tree = field_tree(&records);
                fn keys(tree: &[forge_core::boxes::FieldNode], out: &mut Vec<String>) {
                    for node in tree {
                        if node.children.is_empty() {
                            out.push(node.path.join("."))
                        } else {
                            keys(&node.children, out)
                        }
                    }
                }
                let mut valid = Vec::new();
                keys(&tree, &mut valid);
                request.ticked.retain(|key| valid.contains(key));
                if request.ticked.is_empty() {
                    request.ticked = default_ticks(&tree)
                }
                request.records_path = path;
                summary.count = Some(records.len() as f64);
                summary.fields = Some(tree);
                summary.sample = Some(json.into());
            } else {
                summary.ok = false;
                summary.error = Some("The response isn't JSON, so there are no fields to pick.".into());
            }
        }
        self.status = summary.error.clone().unwrap_or_else(|| {
            format!(
                "{} {} · {:.0} ms · {} records",
                summary.status.unwrap_or_default(),
                summary.status_text.as_deref().unwrap_or(""),
                result.ms,
                summary.count.unwrap_or_default()
            )
        });
        request.test = Some(summary);
        self.refresh_response();
        cx.notify();
    }
    fn refresh_response(&mut self) {
        use forge_core::api_fields::csv_from_sample;
        self.response_preview = self
            .current()
            .and_then(|b| b.requests.get(self.request))
            .and_then(|r| {
                let json = r.test.as_ref()?.sample.as_ref()?;
                Some(csv_from_sample(json, r.records_path.as_deref(), &r.ticked, 10).0)
            })
            .unwrap_or_default();
    }
    fn response_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.global::<ForgeTheme>().0.clone();
        let mut view = div().flex().flex_col().gap(t.px(4.0));
        if let Some(request) = self.current().and_then(|b| b.requests.get(self.request)) {
            if let Some(test) = &request.test {
                if let Some(json) = &test.sample {
                    for (i, list) in forge_core::api_fields::record_lists(json).into_iter().enumerate() {
                        let chosen = request.records_path.as_ref() == Some(&list.path);
                        let label = format!(
                            "{} {} ({} records)",
                            if chosen { "●" } else { "○" },
                            if list.path.is_empty() { "Response".into() } else { list.path.join(".") },
                            list.count
                        );
                        view = view.child(div().id(("connect-records", i)).cursor_pointer().child(label).on_click(
                            cx.listener(move |p, _, _, cx| {
                                if let Some(r) = p
                                    .selected
                                    .and_then(|i| p.boxes.get_mut(i))
                                    .and_then(|b| b.requests.get_mut(p.request))
                                {
                                    r.records_path = Some(list.path.clone());
                                    if let Some(test) = &mut r.test {
                                        if let Some(json) = &test.sample {
                                            let (records, _) =
                                                forge_core::api_fields::pick_records(json, r.records_path.as_deref());
                                            let fields = forge_core::api_fields::field_tree(&records);
                                            r.ticked = forge_core::api_fields::default_ticks(&fields);
                                            test.fields = Some(fields);
                                            test.count = Some(records.len() as f64);
                                        }
                                    }
                                }
                                p.refresh_response();
                                cx.emit(ConnectEvent::Changed);
                                cx.notify();
                            }),
                        ));
                    }
                }
                fn leaves<'a>(
                    nodes: &'a [forge_core::boxes::FieldNode],
                    out: &mut Vec<&'a forge_core::boxes::FieldNode>,
                ) {
                    for n in nodes {
                        if n.children.is_empty() {
                            out.push(n)
                        } else {
                            leaves(&n.children, out)
                        }
                    }
                }
                let mut fields = Vec::new();
                if let Some(tree) = &test.fields {
                    leaves(tree, &mut fields)
                }
                for (i, field) in fields.iter().enumerate() {
                    let selector = format!("connect-response-field-{i}");
                    let key = field.path.join(".");
                    let checked = request.ticked.contains(&key);
                    view = view.child(
                        div()
                            .id(("connect-response-field", i))
                            .debug_selector(move || selector.clone())
                            .cursor_pointer()
                            .child(format!("{} {} · {}", if checked { "☑" } else { "☐" }, key, field.sample))
                            .on_click(cx.listener(move |p, _, _, cx| {
                                if let Some(r) = p
                                    .selected
                                    .and_then(|i| p.boxes.get_mut(i))
                                    .and_then(|b| b.requests.get_mut(p.request))
                                {
                                    if r.ticked.contains(&key) {
                                        r.ticked.retain(|k| k != &key)
                                    } else {
                                        r.ticked.push(key.clone());
                                    }
                                }
                                p.refresh_response();
                                cx.emit(ConnectEvent::Changed);
                                cx.notify();
                            })),
                    );
                }
            }
        }
        if !self.response_preview.is_empty() {
            view = view
                .child(div().text_color(t.fg_muted).child("CSV preview (first 10 records)"))
                .child(div().child(self.response_preview.clone()));
        }
        view.into_any_element()
    }
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut inputs = HashMap::new();
        let mut subscriptions = Vec::new();
        for (field, _, _, multi) in FIELDS {
            let input = if multi {
                let input = cx.new(|cx| TextareaState::new(window, cx));
                subscriptions.push(cx.subscribe_in(&input, window, move |p, _, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::Change) {
                        p.changed(field, window, cx);
                    }
                }));
                FieldInput::Many(input)
            } else {
                let placeholder = match field {
                    Field::Url => "https://api.example.com/users",
                    Field::Base => "https://api.example.com",
                    Field::Target => "api/client.py",
                    Field::Output => "results.csv",
                    Field::Function => "Use the generated name",
                    _ => "",
                };
                let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
                subscriptions.push(cx.subscribe_in(&input, window, move |p, _, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::Change) {
                        p.changed(field, window, cx);
                    }
                }));
                FieldInput::Line(input)
            };
            inputs.insert(field, input);
        }
        Self {
            boxes: Vec::new(),
            response_preview: String::new(),
            selected: None,
            request: 0,
            has_project: false,
            status: String::new(),
            show_api: false,
            show_advanced: false,
            show_code: false,
            testing: false,
            env_ignored: false,
            inputs,
            _subscriptions: subscriptions,
        }
    }
    pub fn current(&self) -> Option<&ApiBox> {
        self.selected.and_then(|i| self.boxes.get(i))
    }
    pub fn set_boxes(&mut self, boxes: Vec<ApiBox>, has_project: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.boxes = boxes;
        self.selected = (!self.boxes.is_empty()).then_some(0);
        self.request = 0;
        self.has_project = has_project;
        self.status.clear();
        self.testing = false;
        self.refresh_form(window, cx);
        cx.notify();
    }
    pub fn add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.has_project {
            return;
        }
        let ids = self.boxes.iter().map(|b| b.id.clone()).collect::<Vec<_>>();
        let mut name = "API".to_owned();
        let mut next = 2;
        while self.boxes.iter().any(|b| b.name == name) {
            name = format!("API {next}");
            next += 1;
        }
        self.boxes.push(new_api_box(&ids, &name));
        self.select(self.boxes.len() - 1, window, cx);
        cx.emit(ConnectEvent::Changed);
    }
    pub fn select(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.boxes.len() {
            return;
        }
        self.selected = Some(index);
        self.request = 0;
        self.refresh_form(window, cx);
        cx.emit(ConnectEvent::Selected);
        cx.notify();
    }
    pub fn method(&mut self, method: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(i) = self.selected {
            if let Some(r) = self.boxes.get_mut(i).and_then(|b| b.requests.get_mut(self.request)) {
                r.method = method.into();
                self.refresh_form(window, cx);
                cx.emit(ConnectEvent::Changed);
            }
        }
    }
    pub fn auth(&mut self, kind: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(b) = self.selected.and_then(|i| self.boxes.get_mut(i)) {
            b.auth = match kind {
                "bearer" => BoxAuth::Bearer { env: forge_core::boxes::env_name(&b.name, "TOKEN") },
                "header" => {
                    BoxAuth::Header { name: "X-Api-Key".into(), env: forge_core::boxes::env_name(&b.name, "KEY") }
                }
                "query" => BoxAuth::Query { name: "api_key".into(), env: forge_core::boxes::env_name(&b.name, "KEY") },
                "basic" => BoxAuth::Basic {
                    user_env: forge_core::boxes::env_name(&b.name, "USER"),
                    pass_env: forge_core::boxes::env_name(&b.name, "PASSWORD"),
                },
                _ => BoxAuth::None,
            };
            self.refresh_form(window, cx);
            cx.emit(ConnectEvent::Changed);
        }
    }
    fn value(&self, field: Field, cx: &App) -> String {
        match &self.inputs[&field] {
            FieldInput::Line(input) => input.read(cx).value().to_string(),
            FieldInput::Many(input) => input.read(cx).value().to_string(),
        }
    }
    pub fn refresh_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.refresh_response();
        let Some(b) = self.current() else {
            return;
        };
        let Some(r) = b.requests.get(self.request) else {
            return;
        };
        let kv = |values: &[KeyValue]| {
            values
                .iter()
                .map(|v| format!("{}{}={}", if v.arg { "?" } else { "" }, v.key, v.value))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let env = auth_env(b);
        let auth_name = match &b.auth {
            BoxAuth::Header { name, .. } | BoxAuth::Query { name, .. } => name.clone(),
            _ => String::new(),
        };
        let values = [
            (Field::Name, b.name.clone()),
            (Field::Base, b.base_url.clone()),
            (Field::Target, b.target.file.clone()),
            (Field::Request, r.name.clone()),
            (Field::Url, r.url.clone()),
            (Field::Output, r.output.clone()),
            (Field::Function, r.function_name.clone().unwrap_or_default()),
            (Field::Body, r.body.clone()),
            (Field::Query, kv(&r.query)),
            (Field::Headers, kv(&r.headers)),
            (Field::Args, r.test_args.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("\n")),
            (Field::Env, env.first().cloned().unwrap_or_default()),
            (Field::AuthName, auth_name),
            (Field::UserEnv, env.first().cloned().unwrap_or_default()),
            (Field::PassEnv, env.get(1).cloned().unwrap_or_default()),
        ];
        for (field, value) in values {
            if self.value(field, cx) == value {
                continue;
            }
            match &self.inputs[&field] {
                FieldInput::Line(input) => input.update(cx, |i, cx| i.set_value(value, window, cx)),
                FieldInput::Many(input) => input.update(cx, |i, cx| i.set_value(value, window, cx)),
            }
        }
    }
    fn changed(&mut self, field: Field, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.value(field, cx);
        let Some(index) = self.selected else {
            return;
        };
        let Some(api) = self.boxes.get_mut(index) else {
            return;
        };
        let string_change = |target: &mut String| {
            if *target == value {
                false
            } else {
                *target = value.clone();
                true
            }
        };
        let changed = match field {
            Field::Name => {
                if api.name == value {
                    false
                } else {
                    rename_box(api, &value);
                    true
                }
            }
            Field::Base => string_change(&mut api.base_url),
            Field::Target => string_change(&mut api.target.file),
            Field::Env => match &mut api.auth {
                BoxAuth::Header { env, .. } | BoxAuth::Query { env, .. } | BoxAuth::Bearer { env } => {
                    string_change(env)
                }
                _ => false,
            },
            Field::AuthName => match &mut api.auth {
                BoxAuth::Header { name, .. } | BoxAuth::Query { name, .. } => string_change(name),
                _ => false,
            },
            Field::UserEnv => match &mut api.auth {
                BoxAuth::Basic { user_env, .. } => string_change(user_env),
                _ => false,
            },
            Field::PassEnv => match &mut api.auth {
                BoxAuth::Basic { pass_env, .. } => string_change(pass_env),
                _ => false,
            },
            _ => {
                let Some(r) = api.requests.get_mut(self.request) else {
                    return;
                };
                match field {
                    Field::Request => string_change(&mut r.name),
                    Field::Url => string_change(&mut r.url),
                    Field::Output => string_change(&mut r.output),
                    Field::Function => {
                        let name = (!value.trim().is_empty()).then(|| value.trim().to_owned());
                        if r.function_name == name {
                            false
                        } else {
                            r.function_name = name;
                            true
                        }
                    }
                    Field::Body => string_change(&mut r.body),
                    Field::Query | Field::Headers => {
                        let values: Vec<_> = value
                            .lines()
                            .filter_map(|l| l.split_once('='))
                            .map(|(key, value)| KeyValue {
                                key: key.trim().trim_start_matches('?').into(),
                                value: value.into(),
                                arg: field == Field::Query && key.trim().starts_with('?'),
                            })
                            .collect();
                        let target = if field == Field::Query { &mut r.query } else { &mut r.headers };
                        if *target == values {
                            false
                        } else {
                            *target = values;
                            true
                        }
                    }
                    Field::Args => {
                        let args = value
                            .lines()
                            .filter_map(|l| l.split_once('='))
                            .map(|(k, v)| (k.trim().into(), v.into()))
                            .collect();
                        if r.test_args == args {
                            false
                        } else {
                            r.test_args = args;
                            true
                        }
                    }
                    _ => false,
                }
            }
        };
        if changed {
            if field == Field::Name {
                self.refresh_form(window, cx);
            }
            cx.emit(ConnectEvent::Changed);
            cx.notify();
        }
    }
    fn render_field(&self, field: Field, id: &'static str, label: &'static str, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.global::<ForgeTheme>().0.clone();
        let input = match &self.inputs[&field] {
            FieldInput::Line(input) => Input::new(input).w_full().into_any_element(),
            FieldInput::Many(input) => Textarea::new(input).h(t.px(72.0)).w_full().into_any_element(),
        };
        div()
            .w_full()
            .min_w_0()
            .flex_none()
            .flex()
            .flex_col()
            .gap(t.px(3.0))
            .child(div().text_color(t.fg_muted).text_size(t.px(11.0)).child(label))
            .child(div().debug_selector(move || format!("connect-field-{id}")).w_full().min_w_0().child(input))
            .into_any_element()
    }
}
impl Render for ConnectPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_settings(cx)
    }
}
pub struct Toolbox {
    pub settings: Entity<ConnectPanel>,
    _subscription: Subscription,
}
#[derive(Clone)]
pub struct ApiBoxDrag;
struct ApiBoxGhost;
impl Render for ApiBoxGhost {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.global::<ForgeTheme>().0.clone();
        div().p(t.px(8.)).bg(t.raised).text_color(t.heat_2).child("API box")
    }
}
impl Toolbox {
    pub fn new(settings: Entity<ConnectPanel>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&settings, |_, _, cx| cx.notify());
        Self { settings, _subscription: subscription }
    }
}
impl Render for Toolbox {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.global::<ForgeTheme>().0.clone();
        let panel = self.settings.read(cx);
        let settings = self.settings.clone();
        let openapi = self.settings.clone();
        let rows = panel
            .boxes
            .iter()
            .enumerate()
            .map(|(i, b)| {
                let settings = self.settings.clone();
                div()
                    .id(("toolbox-api", i))
                    .px(t.px(8.0))
                    .py(t.px(6.0))
                    .cursor_pointer()
                    .text_color(if panel.selected == Some(i) { t.heat_2 } else { t.fg })
                    .hover(|s| s.bg(t.bg_hover))
                    .on_click(move |_, window, cx| settings.update(cx, |p, cx| p.select(i, window, cx)))
                    .child(b.name.clone())
            })
            .collect::<Vec<_>>();
        div()
            .size_full()
            .flex()
            .flex_col()
            .text_size(t.px(12.0))
            .child(
                div()
                    .id("connect-add-box")
                    .debug_selector(|| "connect-add-box".into())
                    .px(t.px(8.0))
                    .py(t.px(8.0))
                    .cursor_pointer()
                    .text_color(t.heat_2)
                    .on_drag(ApiBoxDrag, |_, _, _, cx| cx.new(|_| ApiBoxGhost))
                    .on_click(move |_, _, cx| settings.update(cx, |_, cx| cx.emit(ConnectEvent::Add)))
                    .child("+ API box"),
            )
            .children(rows)
            .children(["Database", "File", "Schedule"].into_iter().map(|name| {
                div()
                    .px(t.px(8.))
                    .py(t.px(5.))
                    .text_color(t.fg_muted)
                    .text_size(t.px(11.))
                    .child(format!("{name} · coming later"))
            }))
            .child(
                div()
                    .id("connect-openapi")
                    .debug_selector(|| "connect-openapi".into())
                    .p(t.px(8.))
                    .cursor_pointer()
                    .text_color(t.heat_2)
                    .child("+ API from OpenAPI…")
                    .on_click(move |_, _, cx| openapi.update(cx, |_, cx| cx.emit(ConnectEvent::OpenApi))),
            )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[gpui_kit::test]
    fn request_basics_and_actions_stay_visible_while_optional_sections_preserve_values(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_global(ForgeTheme(std::sync::Arc::new(crate::theme::Theme::molten())));
        });
        let mut panel = None;
        let (_, cx) = cx.add_window_view(|window, cx| {
            let p = cx.new(|cx| ConnectPanel::new(window, cx));
            panel = Some(p.clone());
            gpui_kit::component::Root::new(p, window, cx)
        });
        let panel = panel.unwrap();
        panel.update_in(cx, |p, window, cx| {
            let mut api = new_api_box(&[], "Weather");
            api.requests[0].headers =
                vec![KeyValue { key: "X-Mode".into(), value: "full".into(), ..Default::default() }];
            api.requests[0].function_name = Some("weather_now".into());
            p.set_boxes(vec![api], true, window, cx);
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("connect-field-url").is_some());
        assert!(cx.debug_bounds("connect-field-headers").is_none());
        assert!(cx.debug_bounds("connect-field-target").is_none());
        let before = cx.debug_bounds("connect-test-call").expect("Test stays in the action footer");
        let advanced = cx.debug_bounds("connect-advanced").expect("Optional request settings are collapsed");
        cx.simulate_click(advanced.center(), gpui_kit::Modifiers::default());
        cx.run_until_parked();
        assert!(cx.debug_bounds("connect-field-headers").is_some());
        assert_eq!(
            before.origin,
            cx.debug_bounds("connect-test-call").unwrap().origin,
            "Expanding options must not move the primary action"
        );
        cx.simulate_click(advanced.center(), gpui_kit::Modifiers::default());
        cx.run_until_parked();
        panel.read_with(cx, |p, _| {
            assert_eq!(p.boxes[0].requests[0].headers[0].value, "full");
            assert_eq!(p.boxes[0].requests[0].function_name.as_deref(), Some("weather_now"));
        });
    }
}
