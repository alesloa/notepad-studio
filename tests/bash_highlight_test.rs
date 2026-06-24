use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

const HIGHLIGHT_NAMES: &[&str] = &[
    "attribute",
    "comment",
    "constant",
    "constant.builtin",
    "constructor",
    "function",
    "function.builtin",
    "function.macro",
    "function.method",
    "keyword",
    "label",
    "number",
    "operator",
    "property",
    "punctuation",
    "punctuation.bracket",
    "punctuation.delimiter",
    "punctuation.special",
    "string",
    "string.special",
    "tag",
    "type",
    "type.builtin",
    "variable",
    "variable.builtin",
    "variable.parameter",
];

#[test]
fn bash_emits_events() {
    let config_result = HighlightConfiguration::new(
        tree_sitter_bash::LANGUAGE.into(),
        "bash",
        tree_sitter_bash::HIGHLIGHT_QUERY,
        "",
        "",
    );
    let mut config = match config_result {
        Ok(c) => {
            println!("BASH CONFIG OK");
            c
        }
        Err(e) => {
            println!("BASH CONFIG ERR: {:?}", e);
            panic!("bash config failed: {:?}", e);
        }
    };
    config.configure(HIGHLIGHT_NAMES);

    let source = "#!/bin/bash\nx=1\necho \"hi\"\nif [ -d foo ]; then echo y; fi\n# comment\n";
    let mut hl = Highlighter::new();
    let events = hl
        .highlight(&config, source.as_bytes(), None, |_| None)
        .expect("highlight call failed");

    let mut count = 0usize;
    let mut stack: Vec<usize> = Vec::new();
    for ev in events {
        match ev.expect("event err") {
            HighlightEvent::Source { start, end } => {
                if let Some(&idx) = stack.last() {
                    let name = HIGHLIGHT_NAMES.get(idx).copied().unwrap_or("?");
                    println!("SRC {}..{} = {} ({:?})", start, end, name, &source[start..end]);
                    count += 1;
                }
            }
            HighlightEvent::HighlightStart(h) => {
                stack.push(h.0);
            }
            HighlightEvent::HighlightEnd => {
                stack.pop();
            }
        }
    }
    println!("TOTAL HIGHLIGHTED RANGES: {}", count);
    assert!(count > 0, "bash produced zero highlight ranges");
}
