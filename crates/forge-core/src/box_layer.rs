//! Connect-box anchors, labels and collision-free placement, replacing the
//! pure parts of `renderer/graph/boxLayer.ts`. Native drawing lives in forge-app.
use crate::{
    box_code::request_functions,
    boxes::{ApiBox, ApiRequest, FieldNode},
    graph_layout::{Layout, Placed},
    graph_view::GraphMode,
};
use std::collections::BTreeMap;
#[derive(Clone, Debug)]
pub struct PlacedBox {
    pub rect: Placed,
    pub anchor: Option<String>,
}
pub fn box_anchor(api: &ApiBox, mode: GraphMode, layout: &Layout) -> Option<String> {
    let module = &api.attach.as_ref()?.module;
    if mode != GraphMode::Modules {
        for name in request_functions(api).values() {
            let id = format!("{module}:{name}");
            if layout.nodes.contains_key(&id) {
                return Some(id);
            }
        }
    }
    layout.nodes.contains_key(module).then(|| module.clone())
}
pub fn box_title(api: &ApiBox) -> String {
    if api.name.split(|c: char| !c.is_ascii_alphanumeric() && c != '_').any(|s| s.eq_ignore_ascii_case("api")) {
        api.name.clone()
    } else {
        format!("{} · API", api.name)
    }
}
pub fn box_summary(api: &ApiBox, request: &ApiRequest) -> String {
    let mut url = request.url.trim().to_owned();
    if !url.is_empty()
        && !url.to_ascii_lowercase().starts_with("http://")
        && !url.to_ascii_lowercase().starts_with("https://")
        && !api.base_url.trim().is_empty()
    {
        url = format!("{}/{}", api.base_url.trim().trim_end_matches('/'), url.trim_start_matches('/'));
    }
    if url.to_ascii_lowercase().starts_with("https://") {
        url = url[8..].into()
    } else if url.to_ascii_lowercase().starts_with("http://") {
        url = url[7..].into()
    }
    if url.is_empty() {
        url = "(no URL yet)".into()
    }
    fn leaves(fields: &[FieldNode]) -> usize {
        fields.iter().map(|f| if f.kind == "object" { leaves(&f.children) } else { 1 }).sum()
    }
    let count = request.ticked.len();
    let tail = if let Some(fields) = request.test.as_ref().and_then(|t| t.fields.as_ref()).filter(|f| !f.is_empty()) {
        format!("{count} of {} fields", leaves(fields))
    } else if count > 0 {
        format!("{count} field{}", if count == 1 { "" } else { "s" })
    } else {
        "not tested".into()
    };
    format!("{} {url} → {} · {tail}", request.method, request.output)
}
fn size(api: &ApiBox) -> (f64, f64) {
    let longest = api
        .requests
        .iter()
        .map(|r| box_summary(api, r).encode_utf16().count())
        .chain(std::iter::once(api.name.encode_utf16().count() + 6))
        .max()
        .unwrap_or(0);
    ((longest as f64 * 6.3 + 34.).clamp(220., 460.).round(), 26. + 18. * api.requests.len() as f64 + 6.)
}
pub fn place_boxes(layout: &Layout, boxes: &[ApiBox], mode: GraphMode) -> BTreeMap<String, PlacedBox> {
    let mut ordered: Vec<_> = boxes.iter().map(|b| (b, box_anchor(b, mode, layout))).collect();
    ordered.sort_by(|(_, a), (_, b)| {
        let a = a.as_ref().and_then(|id| layout.nodes.get(id));
        let b = b.as_ref().and_then(|id| layout.nodes.get(id));
        match (a, b) {
            (Some(a), Some(b)) => a.y.total_cmp(&b.y).then(a.x.total_cmp(&b.x)),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            _ => std::cmp::Ordering::Equal,
        }
    });
    let mut out: BTreeMap<String, PlacedBox> = BTreeMap::new();
    let mut loose = Vec::new();
    for (api, anchor) in ordered {
        let Some(node) = anchor.as_ref().and_then(|id| layout.nodes.get(id)) else {
            loose.push(api);
            continue;
        };
        let (width, height) = size(api);
        let mut rect = Placed { x: node.x + node.width + 24., y: node.y, width, height, layer: node.layer };
        // Jump below a collision; each step clears at least one earlier box.
        while let Some(hit) = out.values().map(|p| &p.rect).find(|p| {
            rect.x < p.x + p.width
                && p.x < rect.x + rect.width
                && rect.y < p.y + p.height + 8.
                && p.y < rect.y + rect.height + 8.
        }) {
            rect.y = hit.y + hit.height + 8.;
        }
        out.insert(api.id.clone(), PlacedBox { rect, anchor });
    }
    let right = out.values().map(|p| p.rect.x + p.rect.width).fold(layout.width, f64::max);
    let mut y = 0.;
    for api in loose {
        let (width, height) = size(api);
        out.insert(
            api.id.clone(),
            PlacedBox { rect: Placed { x: right + 60., y, width, height, layer: 0 }, anchor: None },
        );
        y += height + 8.;
    }
    out
}
pub fn box_bounds(layout: &Layout, placed: &BTreeMap<String, PlacedBox>) -> (f64, f64) {
    placed.values().fold((layout.width, layout.height), |(w, h), p| {
        (w.max(p.rect.x + p.rect.width), h.max(p.rect.y + p.rect.height))
    })
}
/// Add the Connect layer after the project layout, so boxes never alter the
/// dependency layout or detach from their source node during a rebuild.
pub fn add_boxes(data: &mut crate::graph_view::GraphViewData, layout: &mut Layout, boxes: &[ApiBox], mode: GraphMode) {
    let placed = place_boxes(layout, boxes, mode);
    let (width, height) = box_bounds(layout, &placed);
    for api in boxes {
        let Some(p) = placed.get(&api.id) else { continue };
        let id = format!("box:{}", api.id);
        data.nodes.push(crate::graph_view::ViewNode {
            id: id.clone(),
            label: box_title(api),
            detail: api.requests.iter().map(|r| box_summary(api, r)).collect::<Vec<_>>().join(" · "),
            kind: "box".into(),
            box_rows: api
                .requests
                .iter()
                .map(|r| crate::graph_view::BoxRow { text: box_summary(api, r), ok: r.test.as_ref().map(|t| t.ok) })
                .collect(),
            error: api.requests.iter().any(|r| r.test.as_ref().is_some_and(|t| !t.ok)),
            ..Default::default()
        });
        layout.nodes.insert(id.clone(), p.rect);
        if let Some(anchor) = &p.anchor {
            data.edges.push(crate::graph_view::ViewEdge {
                from: anchor.clone(),
                to: id,
                kind: "box".into(),
                ..Default::default()
            });
        }
    }
    layout.width = width;
    layout.height = height;
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        boxes::{new_api_box, BoxAttach},
        graph_layout::{Layout, Placed},
        graph_view::GraphMode,
    };
    use std::collections::BTreeMap;
    fn api() -> crate::boxes::ApiBox {
        let mut b = new_api_box(&[], "GitHub");
        b.attach = Some(BoxAttach { module: "app.api".into() });
        b.base_url = "https://api.github.com".into();
        b.requests[0].name = "users".into();
        b.requests[0].url = "/users".into();
        b.requests[0].output = "users.csv".into();
        b
    }
    fn layout() -> Layout {
        Layout {
            nodes: BTreeMap::from([
                ("app.api".into(), Placed { x: 0., y: 0., width: 100., height: 40., layer: 0 }),
                ("app.api:github_users".into(), Placed { x: 0., y: 100., width: 120., height: 40., layer: 0 }),
            ]),
            width: 300.,
            height: 200.,
        }
    }
    #[test]
    fn anchors_use_module_or_generated_function_in_calls_view() {
        let b = api();
        let l = layout();
        assert_eq!(box_anchor(&b, GraphMode::Modules, &l).as_deref(), Some("app.api"));
        assert_eq!(box_anchor(&b, GraphMode::Calls, &l).as_deref(), Some("app.api:github_users"));
        assert!(box_anchor(&b, GraphMode::Modules, &Layout::default()).is_none());
    }
    #[test]
    fn anchored_boxes_stack_and_loose_boxes_stay_to_the_right() {
        let a = api();
        let mut b = a.clone();
        b.id = "b".into();
        let mut c = b.clone();
        c.id = "c".into();
        c.attach = None;
        let l = layout();
        let p = place_boxes(&l, &[a.clone(), b, c], GraphMode::Modules);
        assert_eq!(p[&a.id].rect.x, 124.);
        assert_eq!(p[&a.id].rect.y, 0.);
        assert!(p["b"].rect.y >= p[&a.id].rect.height + 8.);
        assert!(p["c"].rect.x >= p["b"].rect.x + p["b"].rect.width + 60.);
        let (width, height) = box_bounds(&l, &p);
        assert_eq!(width, p["c"].rect.x + p["c"].rect.width);
        assert!(height >= 200.);
    }
    #[test]
    fn nearby_anchors_never_make_overlapping_boxes() {
        let mut a = api();
        let mut b = a.clone();
        b.id = "b".into();
        b.attach = Some(BoxAttach { module: "other".into() });
        a.requests.extend([a.requests[0].clone(), a.requests[0].clone()]);
        b.requests = a.requests.clone();
        let mut l = layout();
        l.nodes.insert("other".into(), Placed { x: 0., y: 45., width: 100., height: 40., layer: 0 });
        let p = place_boxes(&l, &[a.clone(), b], GraphMode::Modules);
        assert!(p["b"].rect.y >= p[&a.id].rect.y + p[&a.id].rect.height + 8.);
    }
    #[test]
    fn titles_and_summaries_match_the_electron_labels() {
        let mut b = api();
        assert_eq!(box_title(&b), "GitHub · API");
        assert_eq!(box_summary(&b, &b.requests[0]), "GET api.github.com/users → users.csv · not tested");
        b.name = "My api".into();
        assert_eq!(box_title(&b), "My api");
        b.requests[0].ticked = vec!["a".into(), "b".into()];
        assert!(box_summary(&b, &b.requests[0]).ends_with("2 fields"));
        b.requests[0].url.clear();
        assert!(box_summary(&b, &b.requests[0]).contains("(no URL yet)"));
    }
    #[test]
    fn graph_boxes_keep_one_status_row_per_request() {
        let mut api = api();
        api.requests[0].test = Some(crate::boxes::TestSummary { ok: true, ..Default::default() });
        let mut second = crate::boxes::new_request(&api, "repos");
        second.test = Some(crate::boxes::TestSummary { ok: false, ..Default::default() });
        api.requests.push(second);
        let mut data = crate::graph_view::GraphViewData::default();
        let mut layout = layout();
        add_boxes(&mut data, &mut layout, &[api], GraphMode::Modules);
        assert_eq!(data.nodes[0].box_rows.len(), 2);
        assert_eq!(data.nodes[0].box_rows[0].ok, Some(true));
        assert_eq!(data.nodes[0].box_rows[1].ok, Some(false));
    }
}
