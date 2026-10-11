//! Turns `pdftotext -layout` output into clean lines with page-relative indents.
//!
//! The book alternates left/right page margins, so absolute indentation is
//! meaningless; every line gets an indent relative to its page's leftmost line.

use std::sync::LazyLock;

use regex::Regex;

#[derive(Debug, Clone)]
pub struct Line {
    pub page: usize,
    pub indent: usize,
    pub text: String,
}

static FOOTER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\.indd\s+\d+").unwrap());
static PAGE_NO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d{1,3}$").unwrap());

/// Running heads printed at the top of pages. Only dropped when they are the
/// first content line of a page, since some double as real section headings.
const RUNNING_HEADS: &[&str] = &[
    "ESSENTIAL WORDS FOR THE GRE",
    "300 HIGH-FREQUENCY WORD ROOTS",
    "300 HIGH-FREQUENCY GRE WORDS",
    "ANSWER KEYS",
    "PRETEST",
    "POSTTEST",
    "INTRODUCTION: MASTERING ADVANCED VOCABULARY FOR THE GRE",
    "REVIEW: 300 HIGH-FREQUENCY GRE WORDS",
    "REVIEW: ESSENTIAL WORDS FOR THE GRE",
    "COMMON SUFFIXES",
];

pub fn lines(raw: &str) -> Vec<Line> {
    let mut out = Vec::new();
    for (i, page) in raw.split('\u{c}').enumerate() {
        let mut kept: Vec<(usize, String)> = Vec::new();
        for l in page.lines() {
            // pdftotext leaves stray control characters (BEL) from some glyphs.
            let l: String = l.chars().filter(|c| !c.is_control() || *c == '\t').collect();
            let l = l.replace('\t', "  ");
            let t = l.trim();
            if t.is_empty() || FOOTER.is_match(t) || PAGE_NO.is_match(t) {
                continue;
            }
            if kept.is_empty() && RUNNING_HEADS.contains(&t) {
                continue;
            }
            let indent = l.len() - l.trim_start().len();
            kept.push((indent, t.to_string()));
        }
        let base = kept.iter().map(|(n, _)| *n).min().unwrap_or(0);
        out.extend(kept.into_iter().map(|(n, text)| Line { page: i + 1, indent: n - base, text }));
    }
    out
}

/// Joins wrapped lines into one string, undoing end-of-line hyphenation.
/// A hyphen followed by a lowercase letter is treated as a soft break, so a
/// genuine compound split exactly at its hyphen ("self-/interest") loses it;
/// rare in this book and harmless for quizzing.
pub fn join<'a>(parts: impl IntoIterator<Item = &'a str>) -> String {
    let mut s = String::new();
    for p in parts {
        let p = p.trim();
        if p.is_empty() {
            continue;
        }
        if s.ends_with('-') {
            if p.starts_with(|c: char| c.is_lowercase()) {
                s.pop();
            }
        } else if !s.is_empty() {
            s.push(' ');
        }
        s.push_str(p);
    }
    s
}

/// Collapses runs of underscores (answer blanks) into a fixed marker.
pub fn blanks(s: &str) -> String {
    static RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s*_{3,}\s*").unwrap());
    RE.replace_all(s, " ____ ").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_indent_and_footer() {
        let raw = "          ESSENTIAL WORDS FOR THE GRE\n          zephyr n. a gentle breeze\n               A Sample Caption\n          28  \n4-0749_X.indd 28   1/28/16\n\u{c}       lull n. a brief calm\n";
        let ls = lines(raw);
        assert_eq!(ls.len(), 3);
        assert_eq!((ls[0].indent, ls[0].text.as_str()), (0, "zephyr n. a gentle breeze"));
        assert_eq!(ls[1].indent, 5);
        assert_eq!((ls[2].page, ls[2].indent), (2, 0));
    }

    #[test]
    fn joins_hyphenation() {
        assert_eq!(join(["to sus-", "pect that"]), "to suspect that");
        assert_eq!(join(["the Graeco-", "Roman"]), "the Graeco-Roman");
    }
}
