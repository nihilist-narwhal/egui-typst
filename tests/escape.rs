//! `escape` makes arbitrary text inert: shown literally, never run.

use std::error::Error;

use egui_typst::{Compiler, Style, escape};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[test]
fn every_ascii_punctuation_character_is_shown_not_interpreted() -> Result<()> {
    let compiler = Compiler::new()?;
    let style = Style::default();
    for c in (0x21_u8..=0x7e)
        .map(char::from)
        .filter(char::is_ascii_punctuation)
    {
        let alone = format!("a{c}b");
        compiler
            .render(&escape(&alone), &style)
            .map_err(|e| format!("{alone:?}: {e}"))?;
    }
    // a hostile line: code, a file read, math, markup, a reference, a label and a list marker
    let hostile = "#read(\"/etc/passwd\") $x$ *bold* _it_ @ref <label> = heading\n- item\n1. item\n// comment /* block */";
    compiler
        .render(&escape(hostile), &style)
        .map_err(|e| format!("hostile text: {e}"))?;
    // the same text unescaped is an error (it tries to read a file): the escape is what made it safe
    assert!(compiler.render(hostile, &style).is_err());
    Ok(())
}

#[test]
fn escaped_characters_are_drawn() -> Result<()> {
    let compiler = Compiler::new()?;
    let style = Style::default();
    let width = |text: &str| {
        compiler
            .render(&escape(text), &style)
            .map(|image| image.size[0])
    };
    // `#`, `$` and `*` are glyphs here, so each makes the line wider than without it
    assert!(width("a#b")? > width("ab")?);
    assert!(width("a$b")? > width("ab")?);
    assert!(width("a*b")? > width("ab")?);
    Ok(())
}

#[test]
fn escaping_is_the_identity_on_plain_text() {
    assert_eq!(escape("Loss ratio 2026"), "Loss ratio 2026");
    assert_eq!(escape(""), "");
    assert_eq!(escape("é ü ß"), "é ü ß");
}
