//! Layered project-graph layout, replacing `src/renderer/graph/layout.ts`.
//! Sizes in, positions out; independent of scanning, drawing and GPUI.

use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Clone, Debug)]
pub struct LayoutNode {
    pub id: String,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Debug)]
pub struct LayoutEdge {
    pub from: String,
    pub to: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Placed {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// -1 for a node without edges, otherwise its left-to-right column.
    pub layer: i32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layout {
    pub nodes: BTreeMap<String, Placed>,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct LayoutOptions {
    pub gap_x: f64,
    pub gap_y: f64,
    pub sweeps: usize,
}

impl Default for LayoutOptions {
    fn default() -> Self {
        Self { gap_x: 90.0, gap_y: 18.0, sweeps: 6 }
    }
}

pub fn layout_graph(nodes: &[LayoutNode], edges: &[LayoutEdge], options: LayoutOptions) -> Layout {
    let nonnegative = |value: f64| if value.is_finite() { value.max(0.0) } else { 0.0 };
    let gap_x = nonnegative(options.gap_x);
    let gap_y = nonnegative(options.gap_y);
    // Resolve strings once. All graph passes use dense indices and reusable
    // arrays, including the iterative DFS (no call-stack limit on deep graphs).
    let mut by_id = HashMap::with_capacity(nodes.len());
    let mut unique: Vec<&LayoutNode> = Vec::with_capacity(nodes.len());
    for node in nodes {
        if let Some(&index) = by_id.get(node.id.as_str()) {
            unique[index] = node;
        } else {
            by_id.insert(node.id.as_str(), unique.len());
            unique.push(node);
        }
    }
    let count = unique.len();
    let mut out = vec![Vec::new(); count];
    let mut connected = vec![false; count];
    let mut has_in = vec![false; count];
    let mut seen = HashSet::with_capacity(edges.len());
    for edge in edges {
        let (Some(&from), Some(&to)) = (by_id.get(edge.from.as_str()), by_id.get(edge.to.as_str())) else {
            continue;
        };
        if from == to || !seen.insert((from, to)) {
            continue;
        }
        out[from].push(to);
        has_in[to] = true;
        connected[from] = true;
        connected[to] = true;
    }

    // Break cycles by reversing DFS back-edges, preserving the main direction
    // by visiting roots first. Build both adjacency directions in one pass.
    let mut state = vec![0_u8; count];
    let mut succ = vec![Vec::new(); count];
    let mut pred = vec![Vec::new(); count];
    let mut indegree = vec![0_usize; count];
    let mut stack = Vec::new();
    for start in (0..count).filter(|&i| !has_in[i]).chain(0..count) {
        if state[start] != 0 {
            continue;
        }
        state[start] = 1;
        stack.push((start, 0));
        while let Some(&(id, next)) = stack.last() {
            if next == out[id].len() {
                state[id] = 2;
                stack.pop();
                continue;
            }
            // The stack is nonempty (matched above).
            if let Some(top) = stack.last_mut() {
                top.1 += 1;
            }
            let target = out[id][next];
            let (from, to) = if state[target] == 1 { (target, id) } else { (id, target) };
            succ[from].push(to);
            pred[to].push(from);
            indegree[to] += 1;
            if state[target] == 0 {
                state[target] = 1;
                stack.push((target, 0));
            }
        }
    }

    // Longest-path columns in topological order. A cursor avoids removing
    // items from the front of the queue.
    let mut layer = vec![0; count];
    let mut queue: Vec<usize> = (0..count).filter(|&i| connected[i] && indegree[i] == 0).collect();
    let mut cursor = 0;
    while cursor < queue.len() {
        let id = queue[cursor];
        cursor += 1;
        for &target in &succ[id] {
            layer[target] = layer[target].max(layer[id] + 1);
            indegree[target] -= 1;
            if indegree[target] == 0 {
                queue.push(target);
            }
        }
    }
    let column_count = queue.iter().map(|&i| layer[i] + 1).max().unwrap_or(0);
    let mut columns = vec![Vec::new(); column_count];
    for id in 0..count {
        if connected[id] {
            columns[layer[id]].push(id);
        }
    }

    // Neighbour barycentres reduce crossings. Preallocated arrays replace
    // per-column maps and predecessor/successor concatenations.
    let mut rank = vec![0.0; count];
    let mut bary = vec![0.0; count];
    let set_rank = |column: &[usize], ranks: &mut [f64]| {
        let denominator = column.len().saturating_sub(1).max(1) as f64;
        for (position, &id) in column.iter().enumerate() {
            ranks[id] = position as f64 / denominator;
        }
    };
    for column in &columns {
        set_rank(column, &mut rank);
    }
    // Options may come from saved/user input; bound accidental runaway work.
    for sweep in 0..options.sweeps.min(64) {
        let forward = sweep % 2 == 0;
        for step in 0..column_count.saturating_sub(1) {
            let column = &mut columns[if forward { step + 1 } else { column_count - 2 - step }];
            for &id in column.iter() {
                let neighbours = if forward { &pred[id] } else { &succ[id] };
                bary[id] = if neighbours.is_empty() {
                    rank[id]
                } else {
                    neighbours.iter().map(|&n| rank[n]).sum::<f64>() / neighbours.len() as f64
                };
            }
            // Rank breaks ties in the previous order, like JS's stable sort,
            // while using Rust's allocation-free unstable sorting algorithm.
            column.sort_unstable_by(|&a, &b| bary[a].total_cmp(&bary[b]).then_with(|| rank[a].total_cmp(&rank[b])));
            set_rank(column, &mut rank);
        }
    }

    let heights: Vec<f64> = unique.iter().map(|n| nonnegative(n.height)).collect();
    let widths: Vec<f64> = unique.iter().map(|n| nonnegative(n.width)).collect();
    let column_height = |column: &[usize]| {
        column.iter().map(|&id| heights[id]).sum::<f64>() + gap_y * column.len().saturating_sub(1) as f64
    };
    let tallest = columns.iter().map(|c| column_height(c)).fold(0.0_f64, f64::max);
    let mut positions = vec![Placed::default(); count];
    let mut x = 0.0;
    for (index, column) in columns.iter().enumerate() {
        let mut y = (tallest - column_height(column)) / 2.0;
        for &id in column {
            positions[id] = Placed { x, y, width: widths[id], height: heights[id], layer: index as i32 };
            y += heights[id] + gap_y;
        }
        x += column.iter().map(|&id| widths[id]).fold(0.0_f64, f64::max) + gap_x;
    }
    let mut ys = Vec::new();
    for _ in 0..4 {
        for column in &columns {
            ys.clear();
            for &id in column {
                let degree = pred[id].len() + succ[id].len();
                let y = if degree == 0 {
                    positions[id].y
                } else {
                    pred[id].iter().chain(&succ[id]).map(|&n| positions[n].y + heights[n] / 2.0).sum::<f64>()
                        / degree as f64
                        - heights[id] / 2.0
                };
                ys.push(y);
            }
            for i in 1..column.len() {
                ys[i] = ys[i].max(ys[i - 1] + heights[column[i - 1]] + gap_y);
            }
            for i in (0..column.len().saturating_sub(1)).rev() {
                ys[i] = ys[i].min(ys[i + 1] - heights[column[i]] - gap_y);
            }
            for (&id, &y) in column.iter().zip(&ys) {
                positions[id].y = y;
            }
        }
    }
    let min_y = queue.iter().map(|&id| positions[id].y).fold(f64::INFINITY, f64::min);
    let mut height = 0.0_f64;
    let mut width = (x - gap_x).max(0.0);
    for &id in &queue {
        positions[id].y -= min_y;
        height = height.max(positions[id].y + heights[id]);
    }
    let loose: Vec<usize> = (0..count).filter(|&id| !connected[id]).collect();
    if !loose.is_empty() {
        let top = if queue.is_empty() { 0.0 } else { height + gap_y * 3.0 };
        let cell_w = loose.iter().map(|&id| widths[id]).fold(0.0_f64, f64::max) + gap_y;
        let cell_h = loose.iter().map(|&id| heights[id]).fold(0.0_f64, f64::max) + gap_y;
        let grid_columns = (loose.len() as f64).sqrt().ceil();
        let per_row =
            if cell_w > 0.0 { (width.max(cell_w * grid_columns) / cell_w).floor() as usize } else { 1 }.max(1);
        for (index, &id) in loose.iter().enumerate() {
            positions[id] = Placed {
                x: (index % per_row) as f64 * cell_w,
                y: top + (index / per_row) as f64 * cell_h,
                width: widths[id],
                height: heights[id],
                layer: -1,
            };
        }
        width = width.max(loose.len().min(per_row) as f64 * cell_w - gap_y);
        height = top + loose.len().div_ceil(per_row) as f64 * cell_h - gap_y;
    }
    Layout { nodes: unique.iter().enumerate().map(|(i, n)| (n.id.clone(), positions[i])).collect(), width, height }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn node(id: &str, width: f64) -> LayoutNode {
        LayoutNode { id: id.into(), width, height: 24.0 }
    }

    fn edges(pairs: &[(&str, &str)]) -> Vec<LayoutEdge> {
        pairs.iter().map(|(from, to)| LayoutEdge { from: (*from).into(), to: (*to).into() }).collect()
    }

    fn no_overlaps(layout: &Layout) -> bool {
        let all: Vec<_> = layout.nodes.values().collect();
        (0..all.len()).all(|i| {
            (i + 1..all.len()).all(|j| {
                let (a, b) = (all[i], all[j]);
                !(a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height)
            })
        })
    }

    #[test]
    fn puts_what_a_node_points_to_in_a_column_to_its_right() {
        let nodes: Vec<_> = ["main", "service", "util", "db"].map(|id| node(id, 80.0)).into();
        let layout = layout_graph(
            &nodes,
            &edges(&[("main", "service"), ("service", "util"), ("main", "util"), ("service", "db")]),
            LayoutOptions::default(),
        );
        for (id, layer) in [("main", 0), ("service", 1), ("util", 2), ("db", 2)] {
            assert_eq!(layout.nodes[id].layer, layer);
        }
        assert!(layout.nodes["util"].x > layout.nodes["service"].x);
        assert!(no_overlaps(&layout));
    }

    #[test]
    fn handles_loops_without_hanging_still_placing_every_node() {
        let nodes = [node("a", 80.0), node("b", 80.0), node("c", 80.0)];
        let layout = layout_graph(&nodes, &edges(&[("a", "b"), ("b", "c"), ("c", "a")]), LayoutOptions::default());
        assert_eq!(layout.nodes.keys().map(String::as_str).collect::<Vec<_>>(), ["a", "b", "c"]);
        assert_eq!(layout.nodes.values().map(|p| p.layer).collect::<HashSet<_>>().len(), 3);
    }

    #[test]
    fn reduces_crossings_children_follow_the_order_of_their_parents() {
        let nodes: Vec<_> = ["p1", "p2", "c2", "c1"].map(|id| node(id, 80.0)).into();
        let layout = layout_graph(
            &nodes,
            &edges(&[("p1", "c1"), ("p2", "c2"), ("r", "p1"), ("r", "p2")]),
            LayoutOptions::default(),
        );
        assert_eq!(
            (layout.nodes["p1"].y - layout.nodes["p2"].y).signum(),
            (layout.nodes["c1"].y - layout.nodes["c2"].y).signum()
        );
    }

    #[test]
    fn puts_nodes_without_edges_in_a_grid_below_never_overlapping() {
        let mut nodes = vec![node("a", 80.0), node("b", 80.0)];
        nodes.extend((0..10).map(|i| node(&format!("x{i}"), 60.0 + i as f64 * 5.0)));
        let layout = layout_graph(&nodes, &edges(&[("a", "b")]), LayoutOptions::default());
        for n in &nodes[2..] {
            let p = layout.nodes[&n.id];
            assert_eq!(p.layer, -1);
            assert!(p.y > layout.nodes["a"].y + 24.0);
        }
        assert!(no_overlaps(&layout));
        for p in layout.nodes.values() {
            assert!(p.x + p.width <= layout.width + 0.001);
            assert!(p.y + p.height <= layout.height + 0.001);
        }
    }

    #[test]
    fn stays_fast_on_a_big_project_2000_modules() {
        let count = 2000;
        let nodes: Vec<_> = (0..count).map(|i| node(&format!("m{i}"), 100.0)).collect();
        let edges: Vec<_> = (0..count * 3)
            .map(|i| LayoutEdge {
                from: format!("m{}", i * 7919 % count),
                to: format!("m{}", (i * 104729 + 13) % count),
            })
            .collect();
        let start = std::time::Instant::now();
        let layout = layout_graph(&nodes, &edges, LayoutOptions::default());
        let elapsed = start.elapsed();
        eprintln!("2000-module layout: {elapsed:?}");
        assert_eq!(layout.nodes.len(), count);
        assert!(elapsed < std::time::Duration::from_secs(3));
        assert!(no_overlaps(&layout));
    }

    #[test]
    fn empty_graph() {
        assert_eq!(layout_graph(&[], &[], LayoutOptions::default()), Layout::default());
    }

    #[test]
    fn duplicate_unknown_and_self_edges_do_not_change_the_layout() {
        let nodes = [node("a", 80.0), node("b", 80.0)];
        let clean = layout_graph(&nodes, &edges(&[("a", "b")]), LayoutOptions::default());
        let noisy = layout_graph(
            &nodes,
            &edges(&[("a", "a"), ("a", "b"), ("a", "b"), ("missing", "a"), ("b", "missing")]),
            LayoutOptions::default(),
        );
        assert_eq!(clean, noisy);
    }

    #[test]
    fn deep_graph_uses_an_iterative_walk_and_is_deterministic() {
        let nodes: Vec<_> = (0..10_000).map(|i| node(&format!("n{i}"), 80.0)).collect();
        let edges: Vec<_> =
            (1..nodes.len()).map(|i| LayoutEdge { from: nodes[i - 1].id.clone(), to: nodes[i].id.clone() }).collect();
        let first = layout_graph(&nodes, &edges, LayoutOptions::default());
        assert_eq!(first.nodes.len(), nodes.len());
        assert_eq!(first.nodes["n9999"].layer, 9999);
        assert_eq!(layout_graph(&nodes, &edges, LayoutOptions::default()), first);
    }

    #[test]
    fn invalid_sizes_and_zero_sized_loose_nodes_stay_finite() {
        let nodes = [LayoutNode { id: "a".into(), width: f64::NAN, height: f64::INFINITY }, node("b", -10.0)];
        let layout = layout_graph(&nodes, &[], LayoutOptions { gap_x: -10.0, gap_y: 0.0, sweeps: usize::MAX });
        assert!(layout.width.is_finite() && layout.height.is_finite());
        for p in layout.nodes.values() {
            assert!(p.x.is_finite() && p.y.is_finite());
            assert!(p.width >= 0.0 && p.height >= 0.0);
        }
    }
}
