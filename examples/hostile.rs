//! `cargo run --release --example hostile`: what hostile Typst snippets do to a `Compiler`. Each
//! runs on its own thread with a deadline; a snippet that does not finish is reported, and the
//! process exits when the rest is done.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use egui_typst::{Compiler, Style};

const CASES: &[(&str, &str)] = &[
    ("infinite loop", "#while true { }"),
    ("deep recursion", "#let f(n) = f(n + 1)\n#f(0)"),
    ("huge range", "#range(100000000).len()"),
    (
        "big array of strings",
        "#range(200000).map(str).join(\",\")",
    ),
    ("huge repeat", "#str(\"x\" * 1000000).len()"),
    ("ten thousand formulas", "#for i in range(10000) [ $x^#i$ ]"),
    ("very long line", "$ x + y $ #h(1e9pt)"),
    ("read a file", "#read(\"/etc/passwd\")"),
    ("import a package", "#import \"@preview/cetz:0.3.0\""),
    ("huge page", "#box(width: 1e6pt, height: 1e6pt)"),
];

fn main() {
    let (tx, rx) = mpsc::channel();
    for (name, snippet) in CASES {
        let tx = tx.clone();
        let (name, snippet) = ((*name).to_owned(), (*snippet).to_owned());
        std::thread::spawn(move || {
            let started = Instant::now();
            let verdict = match Compiler::new() {
                Err(e) => format!("setup failed: {e}"),
                Ok(compiler) => match compiler.render(&snippet, &Style::default()) {
                    Ok(image) => format!("rendered {}x{} px", image.size[0], image.size[1]),
                    Err(e) => format!(
                        "error: {}",
                        e.to_string().chars().take(70).collect::<String>()
                    ),
                },
            };
            let _ = tx.send((name, verdict, started.elapsed()));
        });
    }
    drop(tx);
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut finished = Vec::new();
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match rx.recv_timeout(left) {
            Ok(done) => finished.push(done),
            Err(_) => break,
        }
    }
    for (name, _) in CASES {
        match finished.iter().find(|f| f.0 == *name) {
            Some((_, verdict, took)) => println!("{name:<24} {:>8.0?}  {verdict}", took),
            None => println!("{name:<24} {:>8}  STILL RUNNING after 20 s", "-"),
        }
    }
    std::process::exit(0);
}
