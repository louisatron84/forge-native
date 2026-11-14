//! Repeatable editor-model latency on a 100,000-line Python buffer (no display timing).
use forge_text::{
    highlighter::Highlighter,
    modes::mode_for,
    text_buffer::{Pos, TextBuffer},
};
use std::time::Instant;
fn main() {
    let source = "    value = compute(123)\n".repeat(100_000);
    let mut buffer = TextBuffer::new(&source);
    let mut highlighter = Highlighter::new();
    let mode = mode_for("stress.py");
    highlighter.tokens_for(&buffer, mode, 50_000);
    let mut times = Vec::new();
    for _ in 0..1000 {
        let start = Instant::now();
        let at = Pos::new(50_000, 4);
        let end = buffer.insert(at, "x");
        for change in buffer.take_changes() {
            highlighter.on_change(&change);
        }
        std::hint::black_box(highlighter.tokens_for(&buffer, mode, 50_000));
        buffer.delete(at, end);
        for change in buffer.take_changes() {
            highlighter.on_change(&change);
        }
        times.push(start.elapsed().as_secs_f64() * 1000.);
    }
    times.sort_by(f64::total_cmp);
    println!(
        "{}",
        serde_json::json!({"model_edit_p95_ms":times[949],"model_edit_max_ms":times[999],"lines":100_000,"iterations":1000})
    );
}
