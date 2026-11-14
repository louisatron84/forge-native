use forge_core::navigation::*;
#[test]
fn treemap_fills_space_proportionally_without_overlap() {
    let items = [6., 6., 4., 3., 2., 2., 1.].iter().enumerate().map(|(i, w)| (i.to_string(), *w)).collect::<Vec<_>>();
    let rects = squarify(&items, Rect { x: 10., y: 20., w: 600., h: 400. });
    for (id, w) in &items {
        let r = &rects[id];
        assert!((r.w * r.h - w / 24. * 240_000.).abs() < 1e-6);
        assert!(r.x >= 10. - 1e-9 && r.y >= 20. - 1e-9 && r.x + r.w <= 610. + 1e-9 && r.y + r.h <= 420. + 1e-9);
    }
    let all = rects.values().collect::<Vec<_>>();
    for (i, a) in all.iter().enumerate() {
        for b in all.iter().skip(i + 1) {
            assert!(
                (a.x + a.w).min(b.x + b.w).sub(a.x.max(b.x)).max(0.)
                    * ((a.y + a.h).min(b.y + b.h) - a.y.max(b.y)).max(0.)
                    < 1e-6
            );
        }
    }
}
#[test]
fn file_map_groups_empty_files_and_path_lookup_handles_slashes() {
    let files =
        vec![("app/a.py".into(), 300), ("app/b.py".into(), 100), ("tests/t.py".into(), 50), ("main.py".into(), 0)];
    let m = project_map(&files, Rect { x: 0., y: 0., w: 800., h: 500. });
    assert_eq!(m.groups.len(), 3);
    assert_eq!(m.files.len(), 4);
    for (path, _) in files {
        let r = m.files[&path];
        let g = m.groups[path.split_once('/').map(|(g, _)| g).unwrap_or("")];
        assert!(r.x >= g.x && r.y >= g.y && r.x + r.w <= g.x + g.w + 1e-8);
    }
}
use std::ops::Sub;
