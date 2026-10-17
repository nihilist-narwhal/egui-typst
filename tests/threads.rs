//! A `Compiler` shared by several threads: each call must typeset its own snippet.

use std::error::Error;

use egui_typst::{Compiler, Style};

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

#[test]
fn a_shared_compiler_typesets_each_threads_own_snippet() -> Result<()> {
    let compiler = Compiler::new().map_err(|e| e.to_string())?;
    // snippets of clearly different widths, so a mix-up shows in the picture's size
    let snippets: Vec<String> = (1..=8)
        .map(|n| format!("$ {} z $", "y + ".repeat(n * 3)))
        .collect();
    let style = Style::default();
    let expected: Vec<[usize; 2]> = snippets
        .iter()
        .map(|s| compiler.render(s, &style).map(|image| image.size))
        .collect::<std::result::Result<_, _>>()
        .map_err(|e| e.to_string())?;
    assert!(
        expected.windows(2).all(|w| w[0] != w[1]),
        "the snippets must differ in size: {expected:?}"
    );

    std::thread::scope(|scope| -> Result<()> {
        let workers: Vec<_> = (0..8)
            .map(|t| {
                let (compiler, snippets, expected, style) =
                    (&compiler, &snippets, &expected, &style);
                scope.spawn(move || -> Result<()> {
                    for i in 0..60 {
                        let k = (t + i) % snippets.len();
                        let got = compiler
                            .render(&snippets[k], style)
                            .map_err(|e| e.to_string())?
                            .size;
                        if got != expected[k] {
                            return Err(format!(
                                "thread {t}, call {i}: snippet {k} came out {got:?}, expected {:?}",
                                expected[k]
                            )
                            .into());
                        }
                    }
                    Ok(())
                })
            })
            .collect();
        for worker in workers {
            worker.join().map_err(|_| "a worker panicked")??;
        }
        Ok(())
    })
}
