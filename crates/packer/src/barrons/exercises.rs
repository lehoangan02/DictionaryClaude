//! Unit review exercises (Matching, Fill-ins, Sense or Nonsense) and the
//! answer keys at the back of the book.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;

use super::text::{blanks, join, Line};

#[derive(Debug, Default, Serialize)]
pub struct Review {
    pub unit: u32,
    /// (number, word)
    pub matching_words: Vec<(u32, String)>,
    /// (letter, definition)
    pub matching_choices: Vec<(char, String)>,
    pub bank: Vec<String>,
    pub fill_ins: Vec<(u32, String)>,
    pub sense: Vec<(u32, String)>,
    /// GRE-style questions closing every tenth unit.
    pub applying: Vec<Question>,
}

#[derive(Debug, Default, Serialize)]
pub struct Question {
    pub number: u32,
    pub stem: String,
    /// Choices per blank; a single-blank question has one list.
    pub blanks: Vec<Vec<String>>,
    pub explanation: String,
    /// The correct choice for each blank, verified against `blanks`.
    pub answers: Vec<String>,
    pub raw_answer: String,
}

#[derive(Debug, Default, Clone, Serialize, PartialEq)]
pub struct Key {
    pub matching: Vec<(u32, char)>,
    pub fill_ins: Vec<(u32, String)>,
    pub sense: Vec<(u32, char)>,
}

static NUMBERED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\d{1,2})\.(?:\s+(.*))?$").unwrap());

/// "3. text" or a bare "3." (the PDF sometimes puts the number alone on a
/// line, with the text on the next one).
pub(super) fn numbered(t: &str) -> Option<(u32, String)> {
    let c = NUMBERED.captures(t)?;
    Some((c[1].parse().ok()?, c.get(2).map_or("", |m| m.as_str()).to_string()))
}
static CHOICE_MARK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\(([A-J])\)\s*").unwrap());

#[derive(Clone, Copy, PartialEq)]
enum Sec {
    None,
    Matching,
    Fill,
    Sense,
}

/// One line of the two-column matching exercise: numbered words on the left,
/// lettered definitions on the right. Wrapped definitions can share a line
/// with a later number, and some choice lines carry no number at all.
pub(super) fn match_line(t: &str, words: &mut Vec<(u32, String)>, choices: &mut Vec<(char, String)>) {
    let (num, tail) = match numbered(t) {
        Some((n, rest)) => (Some(n), rest),
        None => (None, t.to_string()),
    };
    let (left, right): (String, String) = match CHOICE_MARK.find(&tail) {
        Some(m) => (tail[..m.start()].trim().to_string(), tail[m.start()..].to_string()),
        None => match tail.find("  ") {
            Some(i) if num.is_some() => (tail[..i].trim().to_string(), tail[i..].trim().to_string()),
            None if num.is_some() => (tail.trim().to_string(), String::new()),
            _ => (String::new(), tail.trim().to_string()),
        },
    };
    if let (Some(n), false) = (num, left.is_empty()) {
        words.push((n, left));
    }
    if let Some(c) = CHOICE_MARK.captures(&right).filter(|c| c.get(0).unwrap().start() == 0) {
        let end = c.get(0).unwrap().end();
        choices.push((c[1].chars().next().unwrap(), right[end..].trim().to_string()));
    } else if !right.is_empty() {
        if let Some((_, d)) = choices.last_mut() {
            *d = join([d.as_str(), right.as_str()]);
        }
    }
}

pub fn parse_review(unit: u32, lines: &[Line]) -> Review {
    let mut r = Review { unit, ..Default::default() };
    let mut sec = Sec::None;
    let mut seen = [false; 3];
    // Wrapped text of the numbered item being built.
    let mut cur: Option<(u32, Vec<String>)> = None;

    fn finish(r: &mut Review, sec: Sec, cur: &mut Option<(u32, Vec<String>)>) {
        if let Some((n, parts)) = cur.take() {
            let s = blanks(&join(parts.iter().map(String::as_str)));
            match sec {
                Sec::Fill => r.fill_ins.push((n, s)),
                Sec::Sense => r.sense.push((n, s)),
                _ => {}
            }
        }
    }

    for (i, l) in lines.iter().enumerate() {
        let t = l.text.as_str();
        if t.starts_with("APPLYING YOUR KNOWLEDGE") {
            finish(&mut r, sec, &mut cur);
            r.applying = parse_applying(&lines[i + 1..]);
            return r;
        }
        let head = match t {
            "Matching" => Some((Sec::Matching, 0)),
            "Fill-ins" => Some((Sec::Fill, 1)),
            "Sense or Nonsense" => Some((Sec::Sense, 2)),
            _ => None,
        };
        if let Some((next, idx)) = head {
            // A repeated heading means the next book section has begun.
            if seen[idx] {
                break;
            }
            seen[idx] = true;
            finish(&mut r, sec, &mut cur);
            sec = next;
            continue;
        }
        if t.starts_with("The correct answers are")
            || t.starts_with("Match each word")
            || t.starts_with("Choose the best word")
            || t.starts_with("Indicate whether")
            || t.starts_with("Put S (SENSE)")
        {
            continue;
        }
        match sec {
            Sec::Matching => match_line(t, &mut r.matching_words, &mut r.matching_choices),
            Sec::Fill | Sec::Sense => {
                if let Some((n, rest)) = numbered(t) {
                    finish(&mut r, sec, &mut cur);
                    cur = Some((n, vec![rest]));
                } else if let Some((_, parts)) = cur.as_mut() {
                    parts.push(t.to_string());
                } else if sec == Sec::Fill {
                    r.bank.extend(t.split_whitespace().map(String::from));
                }
            }
            Sec::None => {}
        }
    }
    finish(&mut r, sec, &mut cur);
    r
}

static BLANK_HEAD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"Blank \([ivx]+\)").unwrap());

/// Questions of the "Applying Your Knowledge" block: stem, indented choices
/// (columns per blank for multi-blank items), explanation, correct answer.
fn parse_applying(lines: &[Line]) -> Vec<Question> {
    #[derive(PartialEq)]
    enum St {
        Stem,
        Choices,
        Explanation,
        Answer,
    }
    let mut out: Vec<Question> = Vec::new();
    let (mut stem, mut expl, mut ans): (Vec<String>, Vec<&str>, Vec<&str>) = (vec![], vec![], vec![]);
    let mut choice_lines: Vec<&str> = Vec::new();
    let mut st = St::Stem;
    let mut stem_indent = 0;
    let mut num = 0;
    let mut open = false;

    fn close(out: &mut Vec<Question>, num: u32, stem: &mut Vec<String>, choices: &mut Vec<&str>, expl: &mut Vec<&str>, ans: &mut Vec<&str>) {
        let blanks = if choices.first().is_some_and(|l| BLANK_HEAD.is_match(l)) {
            let n = BLANK_HEAD.find_iter(choices[0]).count();
            let mut cols = vec![Vec::new(); n];
            for row in &choices[1..] {
                for (i, cell) in row.split("  ").map(str::trim).filter(|c| !c.is_empty()).enumerate().take(n) {
                    cols[i].push(cell.to_string());
                }
            }
            cols
        } else {
            vec![choices.iter().map(|c| c.trim().to_string()).collect()]
        };
        let raw = join(ans.iter().copied());
        let body = raw
            .trim_start_matches("The correct answers are")
            .trim_start_matches("The correct answer is")
            .trim()
            .trim_end_matches('.')
            .trim()
            .to_string();
        let answers = verify(&blanks, &body);
        out.push(Question {
            number: num,
            stem: blanks_fix(&join(stem.iter().map(String::as_str))),
            blanks,
            explanation: join(expl.iter().copied()),
            answers,
            raw_answer: raw,
        });
        stem.clear();
        choices.clear();
        expl.clear();
        ans.clear();
    }
    fn blanks_fix(s: &str) -> String {
        blanks(s)
    }
    /// Finds the choice per blank whose joined text equals `body`. The book
    /// separates answers with "and", ", and" or ",", and sometimes drops the
    /// leading article of a choice ("anachronism" for "an anachronism").
    fn verify(blanks: &[Vec<String>], body: &str) -> Vec<String> {
        fn bare(c: &str) -> &str {
            ["a ", "an ", "the "].iter().find_map(|a| c.strip_prefix(a)).unwrap_or(c)
        }
        fn go(blanks: &[Vec<String>], body: &str, acc: &mut Vec<String>) -> bool {
            let Some((first, rest)) = blanks.split_first() else { return body.is_empty() };
            for c in first {
                for form in [c.as_str(), bare(c)] {
                    let tail = if rest.is_empty() {
                        body.eq_ignore_ascii_case(form).then_some("")
                    } else {
                        body.strip_prefix(form).and_then(|b| {
                            [", and ", " and ", ", "].iter().find_map(|sep| b.strip_prefix(sep))
                        })
                    };
                    if let Some(tail) = tail {
                        acc.push(c.clone());
                        if go(rest, tail, acc) {
                            return true;
                        }
                        acc.pop();
                    }
                }
            }
            false
        }
        let mut acc = Vec::new();
        if go(blanks, body, &mut acc) { acc } else { Vec::new() }
    }

    for l in lines {
        let t = l.text.as_str();
        // Next book section (the high-frequency review follows unit 80).
        if t == "Matching" || t.starts_with("Review:") {
            break;
        }
        if st != St::Stem || !open {
            if let Some((n, rest)) = numbered(t).filter(|_| matches!(st, St::Stem | St::Answer)) {
                if open {
                    close(&mut out, num, &mut stem, &mut choice_lines, &mut expl, &mut ans);
                }
                open = true;
                num = n;
                stem_indent = l.indent;
                stem.push(rest);
                st = St::Stem;
                continue;
            }
        }
        if !open {
            continue;
        }
        if t.starts_with("Explanation:") {
            st = St::Explanation;
            expl.push(t.trim_start_matches("Explanation:").trim());
        } else if t.starts_with("The correct answer") {
            st = St::Answer;
            ans.push(t);
        } else {
            match st {
                St::Stem if l.indent >= stem_indent + 10 || BLANK_HEAD.is_match(t) => {
                    st = St::Choices;
                    choice_lines.push(t);
                }
                St::Stem => stem.push(t.to_string()),
                St::Choices => choice_lines.push(t),
                St::Explanation => expl.push(t),
                St::Answer => ans.push(t),
            }
        }
    }
    if open {
        close(&mut out, num, &mut stem, &mut choice_lines, &mut expl, &mut ans);
    }
    out
}

static UNIT_KEY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^UNIT (\d{1,2})$").unwrap());
static MARK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:^|\s)(\d{1,2})\.\s+").unwrap());

/// Splits "1. G  2. C ..." into (number, value) pairs; values may be several
/// words (fill-in answers) and cells may be separated by a single space.
pub(super) fn pairs(line: &str) -> Vec<(u32, String)> {
    let marks: Vec<_> = MARK.captures_iter(line).map(|c| (c.get(0).unwrap(), c[1].parse::<u32>().unwrap())).collect();
    let mut out = Vec::new();
    for (i, (m, n)) in marks.iter().enumerate() {
        let end = marks.get(i + 1).map_or(line.len(), |(next, _)| next.start());
        out.push((*n, line[m.end()..end].trim().to_string()));
    }
    out
}

/// Parses the per-unit answer keys. `lines` must start at the first `UNIT`
/// heading of the key section.
pub fn parse_keys(lines: &[Line]) -> BTreeMap<u32, Key> {
    let mut out: BTreeMap<u32, Key> = BTreeMap::new();
    let mut unit = 0;
    let mut sec = Sec::None;
    for l in lines {
        let t = l.text.as_str();
        if let Some(c) = UNIT_KEY.captures(t) {
            unit = c[1].parse().unwrap();
            out.entry(unit).or_default();
            sec = Sec::None;
            continue;
        }
        if unit == 0 {
            continue;
        }
        match t {
            "Matching" => { sec = Sec::Matching; continue }
            "Fill-ins" => { sec = Sec::Fill; continue }
            "Sense or Nonsense" => { sec = Sec::Sense; continue }
            _ => {}
        }
        // Stop at the next book section (Root Roundup answers etc.).
        if t.starts_with("ROOT ROUNDUP") || t.starts_with("ROOT WORK") {
            break;
        }
        let key = out.get_mut(&unit).unwrap();
        for (n, v) in pairs(t) {
            let Some(first) = v.chars().next() else { continue };
            match sec {
                Sec::Matching => key.matching.push((n, first)),
                Sec::Sense => key.sense.push((n, first)),
                Sec::Fill => key.fill_ins.push((n, v)),
                Sec::None => {}
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::barrons::text::lines;

    #[test]
    fn review_unit1() {
        let r = parse_review(
            1,
            &lines(
                "
       Matching
       Match each word with its definition.
        1. dapple                      (A) to scatter widely
        2. verdant                     (B) green with plants
       Fill-ins
       Choose the best word to fill in the blank in each sentence.
          dapple          verdant     fathom
         1.\tThe old captain ____________ the depth of the bay before
             he dropped anchor.
         2.\tThe hillside was ____________ after the long spring rains.
       Sense or Nonsense
         1.\tThe gardener admired how the sun began to dapple the lawn. _____
",
            ),
        );
        assert_eq!(r.matching_words, vec![(1, "dapple".into()), (2, "verdant".into())]);
        assert_eq!(r.matching_choices[1], ('B', "green with plants".into()));
        assert_eq!(r.bank, vec!["dapple", "verdant", "fathom"]);
        assert_eq!(r.fill_ins.len(), 2);
        assert_eq!(r.fill_ins[0].1, "The old captain ____ the depth of the bay before he dropped anchor.");
        assert_eq!(r.sense.len(), 1);
    }

    #[test]
    fn keys() {
        let k = parse_keys(&lines(
            "
          UNIT 1
          Matching
          1. C     2. A    3. B     4. E     5. D    6. F     7. H     8. G  9. J      10. I
          Fill-ins
          1. verdant  2. fathom  3. dapple
          Sense or Nonsense
          1. S     2. N    3. S
",
        ));
        let u = &k[&1];
        assert_eq!(u.matching.len(), 10);
        assert_eq!(u.matching[9], (10, 'I'));
        assert_eq!(u.fill_ins[1], (2, "fathom".into()));
        assert_eq!(u.sense, vec![(1, 'S'), (2, 'N'), (3, 'S')]);
    }
}
