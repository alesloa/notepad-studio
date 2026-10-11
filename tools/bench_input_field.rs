// Standalone optimized benchmark of display preparation, not font shaping.
// rustc --edition 2024 -O tools/bench_input_field.rs -o log/bench_input_field.exe
// log/bench_input_field.exe
use std::{hint::black_box, time::Instant};

#[path = "../src/input_field_window.rs"]
mod text_window;
use text_window::FieldWindow;

fn measure(iterations: usize, prepare: impl Fn() -> String) -> f64 {
    black_box(prepare());
    let start = Instant::now();
    for _ in 0..iterations {
        black_box(prepare());
    }
    start.elapsed().as_secs_f64() * 1_000_000. / iterations as f64
}

fn main() {
    let text = "café中\r\n🙂".repeat(800_000);
    println!(
        "UTF-8 input bytes: {}; optimized display preparation only",
        text.len()
    );
    for (label, cursor) in [
        ("start", 0),
        ("middle", text.len() / 2),
        ("end", text.len()),
    ] {
        // The previous prepaint cloned the full raw value, then replaced its
        // separators in a second allocation. Neither path calls a text system.
        let before = measure(30, || {
            black_box(text.as_str())
                .to_string()
                .replace(['\r', '\n'], " ")
        });
        let after = measure(4000, || {
            let window = FieldWindow::new(black_box(&text), black_box(cursor), black_box(0));
            black_box((
                window.start,
                window.total_len,
                window.cursor,
                window.anchor,
                window.source_byte(window.cursor),
            ));
            window.text.replace(['\r', '\n'], " ")
        });
        let window = FieldWindow::new(&text, cursor, 0);
        println!(
            "{label}: before {before:.2} us, after {after:.2} us, prepared bytes {}",
            window.text.len()
        );
    }
}
