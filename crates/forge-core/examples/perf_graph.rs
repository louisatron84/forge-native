//! Layout-only timing; GPU draw time is deliberately not included.
use forge_core::graph_layout::*;
use std::time::Instant;
fn main() {
    let nodes = (0..400).map(|i| LayoutNode { id: format!("m{i}"), width: 200., height: 90. }).collect::<Vec<_>>();
    let edges =
        (1..400).map(|i| LayoutEdge { from: format!("m{}", (i - 1) / 2), to: format!("m{i}") }).collect::<Vec<_>>();
    let mut times = Vec::new();
    for _ in 0..100 {
        let start = Instant::now();
        std::hint::black_box(layout_graph(&nodes, &edges, LayoutOptions::default()));
        times.push(start.elapsed().as_secs_f64() * 1000.);
    }
    times.sort_by(f64::total_cmp);
    println!(
        "{}",
        serde_json::json!({"layout_400_p95_ms":times[94],"layout_400_max_ms":times[99],"nodes":400,"edges":399,"iterations":100})
    );
}
