//! The pretest and posttest: 20 GRE-format questions each.
//!
//! 1-4   sentence equivalence (pick two of six)
//! 5-14  text completion (one, two or three blanks)
//! 15-20 reading comprehension (passages with numbered lines)

use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::Regex;

use super::exercises::numbered;
use super::extras::{option, ExtraItem, ExtraQuiz};
use super::text::{blanks, join, Line};

/// Lines that are instructions, not question or passage text.
const INSTRUCTIONS: &[&str] = &[
    "Fill in the blank",
    "Fill in all of the blanks",
    "Select one answer choice",
    "Select all",
    "Consider each of the choices",
    "Read the passages",
    "Questions ",
    "Answers that are not",
    "End of ",
    "It\u{2019}s time",
    "It's time",
    "Pretest",
    "Posttest",
    "For the following question",
    "For each of the following",
];

static BLANK_HEAD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"Blank \(([ivx]+)\)").unwrap());
static LINE_MARK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\((\d+)\)\s*").unwrap());

#[derive(Debug, Default, Clone)]
pub struct Raw {
    pub num: u32,
    pub stem: Vec<String>,
    /// Lettered options (Ⓐ / 🄰), in order.
    pub labeled: Vec<String>,
    /// Unlabeled option lines (text completion), raw.
    pub plain: Vec<String>,
    pub passage: Option<String>,
    /// "Select all that apply" was in force for this question.
    pub select_all: bool,
}

fn is_instruction(t: &str) -> bool {
    INSTRUCTIONS.iter().any(|i| t.starts_with(i))
}

/// Reading passages carry line numbers (5), (10), (15)... in sequence. Find
/// each run of markers and the lines around it: four lines precede the first
/// marker, and up to four follow the last. Returns (first line, last line, text).
fn find_passages(lines: &[Line]) -> Vec<(usize, usize, String)> {
    let marker = |i: usize| -> Option<u32> { LINE_MARK.captures(&lines[i].text).and_then(|c| c[1].parse().ok()) };
    let mut groups: Vec<(usize, usize)> = Vec::new(); // (first marker idx, last marker idx)
    let mut prev: Option<(u32, usize)> = None;
    for i in 0..lines.len() {
        let Some(n) = marker(i) else { continue };
        match (prev, groups.last_mut()) {
            (Some((pn, pi)), Some(g)) if n == pn + 5 && i - pi <= 9 => g.1 = i,
            _ if n == 5 => groups.push((i, i)),
            _ => {}
        }
        prev = Some((n, i));
    }
    let stop = |t: &str| is_instruction(t) || option(t).is_some() || numbered(t).is_some_and(|(n, _)| (15..=20).contains(&n));
    let mut out = Vec::new();
    for (f, l) in groups {
        let mut start = f.saturating_sub(4);
        // A "The following is..." title (one or two lines) may sit above the text.
        for back in 1..=3 {
            if start >= back && lines[start - back].text.starts_with("The following is") {
                start -= back;
                break;
            }
        }
        let mut end = l;
        while end < l + 4 && end + 1 < lines.len() && !stop(&lines[end + 1].text) {
            end += 1;
        }
        let text = lines[start..=end].iter().map(|x| x.text.as_str()).collect::<Vec<_>>().join("\n");
        out.push((start, end, text));
    }
    out
}

/// Questions in order, each with its stem, options and passage.
pub fn parse_questions(lines: &[Line]) -> Vec<Raw> {
    let mut out: Vec<Raw> = Vec::new();
    let mut block: Vec<&Line> = Vec::new(); // text between questions: passage or instructions
    let mut passage: Option<String> = None;
    let mut select_all = false;
    let mut last_opt_indent: Option<usize> = None;

    // Passages are the long blocks that sit between two questions.
    let mut close_block = |block: &mut Vec<&Line>, passage: &mut Option<String>, select_all: &mut bool| {
        if block.iter().any(|l| l.text.starts_with("Select all") || l.text.starts_with("Consider each of the choices")) {
            *select_all = true;
        } else if block.iter().any(|l| l.text.starts_with("Select one answer choice")) {
            *select_all = false;
        }
        // Drop instructions, including the lines they wrap onto (until the
        // sentence ends); what remains is passage text.
        let mut text: Vec<&str> = Vec::new();
        let mut in_instruction = false;
        for l in block.iter() {
            let t = l.text.as_str();
            if is_instruction(t) {
                in_instruction = !t.ends_with('.');
            } else if in_instruction {
                in_instruction = !t.ends_with('.');
            } else {
                text.push(t);
            }
        }
        let _ = (&text, &passage);
        block.clear();
    };

    let passages = find_passages(lines);
    let mut idx = 0;
    while idx < lines.len() {
        if let Some((_, end, text)) = passages.iter().find(|p| p.0 == idx) {
            passage = Some(text.clone());
            block.clear();
            last_opt_indent = None;
            idx = end + 1;
            continue;
        }
        let l = &lines[idx];
        idx += 1;
        let t = l.text.as_str();
        // A question number is only trusted when it is the next in sequence.
        let expected = out.last().map_or(1, |q| q.num + 1);
        if let Some((n, rest)) = numbered(t).filter(|(n, _)| *n == expected) {
            close_block(&mut block, &mut passage, &mut select_all);
            last_opt_indent = None;
            out.push(Raw {
                num: n,
                stem: vec![rest],
                passage: if n >= 15 { passage.clone() } else { None },
                select_all,
                ..Default::default()
            });
            continue;
        }
        let Some(q) = out.last_mut() else { continue };
        if let Some((_, text)) = option(t) {
            last_opt_indent = Some(l.indent);
            q.labeled.push(text);
            continue;
        }
        // Wrapped text of the previous labeled option.
        if let (Some(oi), Some(last)) = (last_opt_indent, q.labeled.last_mut()) {
            if l.indent > oi && !is_instruction(t) {
                *last = join([last.as_str(), t]);
                continue;
            }
        }
        if is_instruction(t) {
            block.push(l);
            last_opt_indent = None;
            continue;
        }
        // Without lettered options, the stem runs until a line ends a sentence;
        // the lines after it are the text-completion options.
        if q.labeled.is_empty() && block.is_empty() {
            // Stem and unlabeled options arrive mixed; `split_options` separates them.
            q.stem.push(t.to_string());
        } else {
            block.push(l);
        }
    }
    for q in &mut out {
        split_options(q);
    }
    out
}

/// For text completion, separates the options from the stem: a `Blank (i)`
/// header starts a column block; otherwise the last five lines are the choices.
/// Footnotes (`*A bromide is ...`) after the options are dropped.
fn split_options(q: &mut Raw) {
    if !q.labeled.is_empty() || q.stem.len() < 2 {
        return;
    }
    if let Some(f) = q.stem.iter().position(|l| l.starts_with('*')) {
        q.stem.truncate(f);
    }
    let cut = match q.stem.iter().position(|l| BLANK_HEAD.is_match(l)) {
        Some(h) => h,
        None if q.stem.len() > 5 => q.stem.len() - 5,
        None => return,
    };
    q.plain = q.stem.split_off(cut);
}

/// "1. C, F" / "5. Paradoxically" / "12. a/b/c" rows into number -> value.
pub fn parse_key(lines: &[Line]) -> BTreeMap<u32, String> {
    let mut out: BTreeMap<u32, String> = BTreeMap::new();
    let mut last = 0;
    for l in lines {
        if l.text.starts_with("YOUR ") {
            break;
        }
        match numbered(&l.text) {
            Some((n, rest)) if n == last + 1 => {
                out.insert(n, rest);
                last = n;
            }
            _ => {
                // A quotation answer can wrap onto a second line.
                if let Some(v) = out.get_mut(&last) {
                    if !l.text.starts_with("PRETEST") && !l.text.starts_with("POSTTEST") {
                        *v = join([v.as_str(), &l.text]);
                    }
                }
            }
        }
    }
    out
}

fn norm(s: &str) -> String {
    s.trim().trim_matches(|c: char| matches!(c, '.' | '\u{201C}' | '\u{201D}' | '"')).to_lowercase()
}

/// Splits columns of a text-completion option block into one list per blank.
fn columns(plain: &[String]) -> Option<Vec<Vec<String>>> {
    let head = plain.first()?;
    let n = BLANK_HEAD.find_iter(head).count();
    if n == 0 {
        return None;
    }
    let mut cols = vec![Vec::new(); n];
    for row in &plain[1..] {
        for (i, cell) in row.split("  ").map(str::trim).filter(|c| !c.is_empty()).enumerate().take(n) {
            cols[i].push(cell.to_string());
        }
    }
    Some(cols)
}

/// Close enough to be the same printed word (the book misspells one answer).
fn same_word(a: &str, b: &str) -> bool {
    let (a, b) = (norm(a), norm(b));
    if a == b {
        return true;
    }
    let n = a.chars().count().max(b.chars().count());
    n >= 8 && distance(&a, &b) <= 1
}

fn distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            cur.push((prev[j] + (ca != cb) as usize).min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// The key lists answers in an order that need not match the printed columns,
/// so find which column each answer belongs to (one per column).
fn assign(parts: &[&str], cols: &[Vec<String>]) -> Option<Vec<String>> {
    fn go(parts: &[&str], cols: &[Vec<String>], used: &mut Vec<bool>, out: &mut Vec<Option<String>>, i: usize) -> bool {
        if i == parts.len() {
            return true;
        }
        for (c, list) in cols.iter().enumerate() {
            if used[c] {
                continue;
            }
            if let Some(hit) = list.iter().find(|x| same_word(x, parts[i])) {
                used[c] = true;
                out[c] = Some(hit.clone());
                if go(parts, cols, used, out, i + 1) {
                    return true;
                }
                used[c] = false;
                out[c] = None;
            }
        }
        false
    }
    if parts.len() != cols.len() {
        return None;
    }
    let mut out = vec![None; cols.len()];
    go(parts, cols, &mut vec![false; cols.len()], &mut out, 0).then(|| out.into_iter().flatten().collect())
}

fn sentences(passage: &str) -> Vec<String> {
    // Drop "(5)" line markers and rejoin the wrapped lines.
    let flat = join(passage.lines().map(|l| LINE_MARK.replace(l, "").into_owned()).collect::<Vec<_>>().iter().map(String::as_str));
    let chars: Vec<char> = flat.chars().collect();
    let mut out = Vec::new();
    let mut start = 0;
    for i in 0..chars.len() {
        let end_mark = matches!(chars[i], '.' | '?' | '!') || (matches!(chars[i], '\u{201D}' | '"') && i > 0 && matches!(chars[i - 1], '.' | '?' | '!'));
        let next_starts = chars.get(i + 1).is_some_and(|c| c.is_whitespace()) && chars.get(i + 2).is_some_and(|c| c.is_uppercase() || matches!(c, '\u{201C}' | '"' | '\u{2018}'));
        if end_mark && next_starts {
            let s: String = chars[start..=i].iter().collect();
            if s.split_whitespace().count() >= 3 {
                out.push(s.trim().to_string());
                start = i + 1;
            }
        }
    }
    let tail: String = chars[start..].iter().collect();
    if tail.split_whitespace().count() >= 3 {
        out.push(tail.trim().to_string());
    }
    out
}

pub fn quiz(id: &str, title: &str, raws: &[Raw], key: &BTreeMap<u32, String>, errs: &mut Vec<String>) -> ExtraQuiz {
    let mut items = Vec::new();
    for q in raws {
        let ctx = format!("{id} Q{}", q.num);
        let Some(k) = key.get(&q.num) else {
            errs.push(format!("{ctx}: no answer key"));
            continue;
        };
        let stem = blanks(&join(q.stem.iter().map(String::as_str)));
        let letters = |k: &str| -> Vec<usize> {
            k.split(|c: char| !c.is_alphabetic()).filter(|p| p.len() == 1).filter_map(|p| p.chars().next()).map(|c| (c.to_ascii_uppercase() as u32 - 'A' as u32) as usize).collect()
        };
        let make = |style: &'static str, choices: serde_json::Value, answer: String| ExtraItem {
            ord: q.num,
            style,
            prompt: stem.clone(),
            choices: Some(choices),
            answer,
            explanation: None,
            words: vec![],
            passage: q.passage.clone(),
        };
        let item = if !q.labeled.is_empty() {
            let idx = letters(k);
            let picked: Vec<String> = idx.iter().filter_map(|i| q.labeled.get(*i).cloned()).collect();
            if picked.len() != idx.len() || picked.is_empty() {
                errs.push(format!("{ctx}: key {k:?} does not fit {} options", q.labeled.len()));
                continue;
            }
            if q.num <= 4 || picked.len() > 1 {
                // Sentence equivalence and "select all that apply".
                let style = if q.num <= 4 { "se" } else { "multi" };
                make(style, serde_json::json!(q.labeled), serde_json::json!(picked).to_string())
            } else {
                make("mcq", serde_json::json!(q.labeled), picked[0].clone())
            }
        } else if let Some(cols) = columns(&q.plain) {
            let parts: Vec<&str> = k.split('/').map(str::trim).collect();
            match assign(&parts, &cols) {
                Some(answers) => make("blanks", serde_json::json!(cols), serde_json::json!(answers).to_string()),
                None => {
                    errs.push(format!("{ctx}: key {k:?} does not fit columns {cols:?}"));
                    continue;
                }
            }
        } else if !q.plain.is_empty() {
            let opts: Vec<String> = q.plain.iter().map(|s| s.trim().to_string()).collect();
            match opts.iter().find(|o| norm(o) == norm(k)) {
                Some(a) => make("mcq", serde_json::json!(opts), a.clone()),
                None => {
                    errs.push(format!("{ctx}: key {k:?} not among {opts:?}"));
                    continue;
                }
            }
        } else if stem.to_lowercase().contains("select the sentence") || k.contains("lines") {
            // The answer is a sentence of the passage, cited by its opening words.
            let Some(p) = q.passage.as_deref() else {
                errs.push(format!("{ctx}: sentence question without a passage"));
                continue;
            };
            let all = sentences(p);
            let opening = k.trim_start_matches(|c: char| !c.is_alphanumeric()).split(". . .").next().unwrap_or("").trim().trim_matches('\u{201C}').to_lowercase();
            let hit = all
                .iter()
                .find(|s| s.to_lowercase().trim_start_matches(|c: char| !c.is_alphanumeric()).starts_with(&opening))
                .or_else(|| all.iter().find(|s| s.to_lowercase().contains(&opening)));
            match hit {
                Some(h) if !opening.is_empty() => make("mcq", serde_json::json!(all), h.clone()),
                _ => {
                    let head: String = p.chars().take(80).collect();
                    let tail: String = p.chars().rev().take(120).collect::<Vec<_>>().into_iter().rev().collect();
                    errs.push(format!("{ctx}: cannot find the cited sentence {opening:?} among {} sentences; passage starts {head:?} ends {tail:?}", all.len()));
                    continue;
                }
            }
        } else {
            errs.push(format!("{ctx}: no options found"));
            continue;
        };
        items.push(item);
    }
    ExtraQuiz { id: id.into(), title: title.into(), kind: "test".into(), group: "reviews".into(), items }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::barrons::text::lines;

    #[test]
    fn sentence_equivalence_and_text_completion() {
        let ls = lines(
            "
          1.\tThe old lighthouse was the ____________ of the whole coast.
                 🄰    landmark
                 🄱    relic
                 🄲    symbol
          2.\t____________, the harbor was both busy and silent.
                  Oddly
                  Strangely
                  Plainly
                  Briefly
                  Mostly
          3.\tA twisty stem with ____________ and ____________.
                  Blank (i)        Blank (ii)
                  alpha            gamma
                  beta             delta
",
        );
        let qs = parse_questions(&ls);
        assert_eq!(qs.iter().map(|q| q.num).collect::<Vec<_>>(), vec![1, 2, 3], "{qs:?}");
        assert_eq!(qs[0].labeled, vec!["landmark", "relic", "symbol"]);
        assert_eq!(qs[1].plain.len(), 5);
        assert_eq!(qs[1].plain[1], "Strangely");
        assert_eq!(qs[2].plain.len(), 3);
        assert!(qs[1].labeled.is_empty());
    }

    #[test]
    fn answers_are_resolved_against_options() {
        let mk = |num, labeled: &[&str], plain: &[&str]| Raw { num, stem: vec!["stem ____".into()], labeled: labeled.iter().map(|s| s.to_string()).collect(), plain: plain.iter().map(|s| s.to_string()).collect(), ..Default::default() };
        let raws = vec![
            mk(1, &["a", "b", "c", "d", "e", "f"], &[]),
            mk(2, &[], &["Blank (i)  Blank (ii)", "up  left", "down  right"]),
            mk(3, &[], &["Oddly", "Strangely"]),
        ];
        let key: BTreeMap<u32, String> = [(1, "C, F".to_string()), (2, "left/down".to_string()), (3, "Strangely".to_string())].into();
        let mut errs = Vec::new();
        let q = quiz("t", "T", &raws, &key, &mut errs);
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(q.items[0].style, "se");
        assert_eq!(q.items[0].answer, r#"["c","f"]"#);
        assert_eq!(q.items[1].style, "blanks");
        assert_eq!(q.items[1].answer, r#"["down","left"]"#, "answers follow the printed columns even if the key lists them in another order");
        assert_eq!(q.items[2].answer, "Strangely");
    }

    #[test]
    fn finds_a_sentence_that_follows_a_closing_quotation() {
        let p = "        (55) \u{201C}ships passed in the night.\u{201D} He said it twice: \u{201C}If they\u{2019}re there, you know they\u{2019}re coming, they\u{2019}re coming.\u{201D}\n Still, watch the tide or lose the boat, there are not many sailors around\n who can read the harbor cur-\n rents today. The chart is as much a guide as it is\n (60) a record of the coast.";
        let s = sentences(p);
        assert!(s.iter().any(|x| x.starts_with("Still, watch the tide") && x.ends_with("currents today.")), "{s:#?}");
    }

    #[test]
    fn splits_passage_sentences() {
        let p = "(1) The word \u{201C}harbor\u{201D} is broad. It includes docks.\n(2) Still, visit it if you like. It is not above ridicule.";
        let s = sentences(p);
        assert_eq!(s.len(), 4, "{s:?}");
        assert!(s[2].starts_with("Still, visit it"));
    }
}
