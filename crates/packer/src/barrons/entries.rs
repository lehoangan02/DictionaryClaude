//! Word entries inside a unit: headword line, wrapped definition, indented
//! example sentences, derived-form paragraphs, and the "Terms from the Arts,
//! Sciences, and Social Sciences" glossary boxes.

use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;

use super::text::{join, Line};

#[derive(Debug, Default, Serialize)]
pub struct Entry {
    pub unit: u32,
    pub page: usize,
    pub headword: String,
    pub pos: String,
    /// Marked with `*` in the book: one of the 300 high-frequency words.
    pub high_freq: bool,
    pub definition: String,
    pub examples: Vec<String>,
    /// Derived forms and usage notes ("Aberration is a noun meaning ...").
    pub notes: Vec<Note>,
    pub terms: Vec<(String, String)>,
}

#[derive(Debug, Default, Serialize)]
pub struct Note {
    pub text: String,
    pub examples: Vec<String>,
}

static HEADER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(\*)?([a-zA-Zà-ÿ][a-zà-ÿ'’\- ]*?) ((?:n|v|adj|adv|prep|conj|interj)[.,](?:(?:, ?| or | and )(?:n|v|adj|adv)\.)*)(?: (.*))?$",
    )
    .unwrap()
});
static TERM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([A-Za-zà-ÿ'’ .\-]{1,48}?): (.+)$").unwrap());

/// Indent at or above which an indented line starts a new example sentence;
/// smaller positive indents continue the previous one.
const EXAMPLE_START: usize = 4;

/// Where the wrapped text currently being collected will go.
#[derive(Clone, Copy, PartialEq)]
enum Target {
    Definition,
    Example,
    NoteText,
    NoteExample,
    Term,
}

fn flush(e: &mut Entry, target: Target, buf: &mut Vec<String>) {
    if buf.is_empty() {
        return;
    }
    let s = join(buf.iter().map(String::as_str));
    buf.clear();
    match target {
        Target::Definition => e.definition = join([e.definition.as_str(), s.as_str()]),
        Target::Example => e.examples.push(s),
        Target::NoteText => e.notes.push(Note { text: s, examples: Vec::new() }),
        Target::NoteExample => match e.notes.last_mut() {
            Some(n) => n.examples.push(s),
            None => e.examples.push(s),
        },
        Target::Term => {
            if let Some(c) = TERM.captures(&s) {
                e.terms.push((c[1].trim().to_string(), c[2].trim().to_string()));
            }
        }
    }
}

pub fn parse(unit: u32, lines: &[Line]) -> Vec<Entry> {
    let mut entries: Vec<Entry> = Vec::new();
    let mut target = Target::Definition;
    let mut buf: Vec<String> = Vec::new();

    for l in lines {
        let t = l.text.as_str();
        if l.indent <= 1 {
            if let Some(c) = HEADER.captures(t) {
                if let Some(e) = entries.last_mut() {
                    flush(e, target, &mut buf);
                }
                entries.push(Entry {
                    unit,
                    page: l.page,
                    high_freq: c.get(1).is_some(),
                    headword: c[2].trim().to_string(),
                    // The book misprints "adj," for "adj." (taciturn).
                    pos: match &c[3] {
                        p if !p.contains('.') => format!("{}.", p.trim_end_matches(',')),
                        p => p.to_string(),
                    },
                    ..Default::default()
                });
                target = Target::Definition;
                buf.extend(c.get(4).map(|d| d.as_str().to_string()));
                continue;
            }
        }
        let Some(e) = entries.last_mut() else { continue };
        if t.starts_with("Terms from the Arts") {
            flush(e, target, &mut buf);
            target = Target::Term;
            continue;
        }
        let next = match (l.indent, target) {
            // Flush-left text continues the definition or a note paragraph,
            // otherwise it opens a new derived-form paragraph.
            (0..=1, Target::Definition | Target::NoteText) => None,
            (0..=1, _) => Some(Target::NoteText),
            (_, Target::Term) => TERM.is_match(t).then_some(Target::Term),
            (n, Target::Definition | Target::Example) if n >= EXAMPLE_START => Some(Target::Example),
            (n, Target::NoteText | Target::NoteExample) if n >= EXAMPLE_START => {
                Some(Target::NoteExample)
            }
            _ => None,
        };
        if let Some(next) = next {
            flush(e, target, &mut buf);
            target = next;
        }
        buf.push(t.to_string());
    }
    if let Some(e) = entries.last_mut() {
        flush(e, target, &mut buf);
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::barrons::text::lines;

    const UNIT: &str = "
       dapple v. to mark with spots of color
           The late sun dappled the lake with gold
         and amber light.
                    Terms from the Arts, Sciences, and Social Sciences
          chiaroscuro: contrast of light and dark in a painting, drawing
          or photograph (Italian: light-dark)
       *verdant adj. green with growing plants
           The verdant hills rolled down until they met
         the sea.
       Verdancy is a noun meaning greenness.
            Travelers often praised the verdancy of the valley
         in spring.
       fathom n. a unit of six feet used to measure the depth of water;
       also a measure of rope
            The anchor line was twenty fathoms long.
";

    #[test]
    fn tolerates_misprinted_pos_and_keeps_multi_pos() {
        let es = parse(
            74,
            &lines("\n       *sylvan adj, of the woods\n       foo n., v. a thing; to thing\n"),
        );
        assert_eq!((es[0].headword.as_str(), es[0].pos.as_str()), ("sylvan", "adj."));
        assert_eq!(es[0].definition, "of the woods");
        assert_eq!(es[1].pos, "n., v.");
    }

    #[test]
    fn parses_unit_fragment() {
        let es = parse(1, &lines(UNIT));
        assert_eq!(es.len(), 3);
        assert_eq!(es[0].headword, "dapple");
        assert_eq!(es[0].pos, "v.");
        assert_eq!(es[0].examples.len(), 1);
        assert!(es[0].examples[0].ends_with("amber light."));
        assert_eq!(es[0].terms[0].0, "chiaroscuro");
        assert!(es[0].terms[0].1.ends_with("(Italian: light-dark)"));

        assert!(es[1].high_freq);
        assert_eq!(es[1].notes.len(), 1);
        assert!(es[1].notes[0].text.ends_with("meaning greenness."));
        assert_eq!(es[1].notes[0].examples.len(), 1);

        assert_eq!(es[2].definition, "a unit of six feet used to measure the depth of water; also a measure of rope");
        assert_eq!(es[2].examples.len(), 1);
    }
}
