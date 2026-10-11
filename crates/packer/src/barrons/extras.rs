//! The parts of the book outside the 80 units: the two cumulative reviews,
//! the word-root lists, Root Work exercises and Root Roundup reviews.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;

use super::exercises::{match_line, numbered, pairs};
use super::text::{blanks, join, Line};

/// One matching block: numbered words against lettered definitions.
#[derive(Debug, Default, Clone, Serialize)]
pub struct Part {
    pub name: String,
    pub words: Vec<(u32, String)>,
    pub choices: Vec<(char, String)>,
}

impl Part {
    /// The definition the key letter points at.
    pub fn answer(&self, letter: char) -> Option<&str> {
        self.choices.iter().find(|(c, _)| *c == letter).map(|(_, d)| d.as_str())
    }
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct Mcq {
    pub num: u32,
    pub stem: String,
    pub options: Vec<String>,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct Cumulative {
    pub parts: Vec<Part>,
    pub sense: Vec<(u32, String)>,
    pub mcq: Vec<Mcq>,
}

/// Index of a circled/squared option letter (Ⓐ..Ⓩ, 🄰..🅉) and the text after it.
pub fn option(t: &str) -> Option<(usize, String)> {
    let c = t.chars().next()?;
    let n = c as u32;
    let idx = match n {
        0x24B6..=0x24CF => n - 0x24B6,
        0x1F130..=0x1F149 => n - 0x1F130,
        _ => return None,
    };
    Some((idx as usize, t[c.len_utf8()..].trim().to_string()))
}

#[derive(Clone, Copy, PartialEq)]
enum Sec {
    None,
    Match,
    Sense,
    Fill,
}

const SKIP: &[&str] = &["Match each word", "Indicate whether", "Put S (SENSE)", "Choose the best word"];

pub fn parse_cumulative(lines: &[Line]) -> Cumulative {
    let mut c = Cumulative::default();
    let mut sec = Sec::None;
    let mut cur: Option<(u32, Vec<String>)> = None; // sense item being built
    let mut stem: Option<(u32, Vec<String>)> = None; // mcq stem being built
    let mut options: Vec<String> = Vec::new();

    fn finish_sense(c: &mut Cumulative, cur: &mut Option<(u32, Vec<String>)>) {
        if let Some((n, parts)) = cur.take() {
            c.sense.push((n, blanks(&join(parts.iter().map(String::as_str)))));
        }
    }
    fn finish_mcq(c: &mut Cumulative, stem: &mut Option<(u32, Vec<String>)>, options: &mut Vec<String>) {
        if let Some((n, parts)) = stem.take() {
            c.mcq.push(Mcq { num: n, stem: blanks(&join(parts.iter().map(String::as_str))), options: std::mem::take(options) });
        }
    }

    for l in lines {
        let t = l.text.as_str();
        let next = match t {
            "Matching" => Some(Sec::Match),
            "Sense or Nonsense" => Some(Sec::Sense),
            "Fill-ins" => Some(Sec::Fill),
            _ => None,
        };
        if let Some(n) = next {
            finish_sense(&mut c, &mut cur);
            finish_mcq(&mut c, &mut stem, &mut options);
            sec = n;
            continue;
        }
        if SKIP.iter().any(|s| t.starts_with(s)) {
            continue;
        }
        match sec {
            Sec::Match => {
                if let Some(name) = t.strip_prefix("Part ").filter(|n| n.len() <= 2) {
                    c.parts.push(Part { name: name.to_string(), ..Default::default() });
                    continue;
                }
                if c.parts.is_empty() {
                    c.parts.push(Part { name: "A".into(), ..Default::default() });
                }
                let p = c.parts.last_mut().unwrap();
                match_line(t, &mut p.words, &mut p.choices);
            }
            Sec::Sense => {
                if let Some((n, rest)) = numbered(t) {
                    finish_sense(&mut c, &mut cur);
                    cur = Some((n, vec![rest]));
                } else if let Some((_, parts)) = cur.as_mut() {
                    parts.push(t.to_string());
                }
            }
            Sec::Fill => {
                if let Some((_, text)) = option(t) {
                    options.push(text.replace("...", "..").replace("..", " \u{2026} "));
                } else if let Some((n, rest)) = numbered(t) {
                    finish_mcq(&mut c, &mut stem, &mut options);
                    stem = Some((n, vec![rest]));
                } else if let Some((_, parts)) = stem.as_mut() {
                    parts.push(t.to_string());
                }
            }
            Sec::None => {}
        }
    }
    finish_sense(&mut c, &mut cur);
    finish_mcq(&mut c, &mut stem, &mut options);
    c
}

/// "1. H   11. H   21. N" rows into number -> value.
pub fn flat_key(lines: &[Line]) -> BTreeMap<u32, String> {
    let mut out = BTreeMap::new();
    for l in lines {
        for (n, v) in pairs(&l.text) {
            if !v.is_empty() {
                out.entry(n).or_insert(v);
            }
        }
    }
    out
}

// ---- word roots -----------------------------------------------------------

#[derive(Debug, Default, Clone, Serialize)]
pub struct Root {
    pub forms: String,
    pub meaning: String,
    pub lang: String,
    pub words: Vec<(String, String)>,
}

static BULLET: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[•c]?\s*([A-Z][A-Z0-9/\-' ]*?)\s*\(([^)]+)\)\s*([A-Za-z][A-Za-z/, ]*)?$").unwrap());

/// The "300 high-frequency word roots" lists: `• ROOT (MEANING) Language`
/// followed by `word = definition` lines.
pub fn parse_roots(lines: &[Line]) -> Vec<Root> {
    let mut out: Vec<Root> = Vec::new();
    // A bullet alone on its line: the root heading follows, possibly wrapped
    // over two or three lines.
    let mut heading: Option<String> = None;
    let mut taken = 0;
    for l in lines {
        let mut t = l.text.as_str();
        if let Some(h) = heading.as_mut() {
            if !t.starts_with('•') {
                h.push(' ');
                h.push_str(t);
                taken += 1;
                if let Some(c) = BULLET.captures(h.trim()) {
                    out.push(Root { forms: c[1].trim().to_string(), meaning: c[2].trim().to_lowercase(), lang: c.get(3).map_or(String::new(), |m| m.as_str().trim().to_string()), words: Vec::new() });
                    heading = None;
                } else if taken >= 3 {
                    heading = None;
                }
                continue;
            }
            heading = None;
        }
        if t == "•" {
            heading = Some(String::new());
            taken = 0;
            continue;
        }
        t = t.trim();
        if let Some(rest) = t.strip_prefix('•') {
            if let Some(c) = BULLET.captures(rest.trim()) {
                out.push(Root { forms: c[1].trim().to_string(), meaning: c[2].trim().to_lowercase(), lang: c.get(3).map_or(String::new(), |m| m.as_str().trim().to_string()), words: Vec::new() });
            }
            continue;
        }
        let Some(r) = out.last_mut() else { continue };
        if t.starts_with("ROOT ROUNDUP") || t.starts_with("Root Work") || t.starts_with("(Answers") {
            continue;
        }
        if let Some((w, d)) = t.split_once(" = ") {
            r.words.push((w.trim().to_string(), d.trim().to_string()));
        } else if let Some((_, d)) = r.words.last_mut() {
            // Wrapped definition (only while still inside the list).
            if l.indent > 0 && !t.contains(" ___") && !numbered(t).is_some() && !t.starts_with("Match") && d.len() < 200 {
                *d = join([d.as_str(), t]);
            }
        }
    }
    out.retain(|r| !r.words.is_empty());
    out
}

static ROOT_WORK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^Root Work (\d+)$").unwrap());
static ROOT_REVIEW: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^ROOT ROUNDUP REVIEW (\d+)[\u{2013}\-](\d+)$").unwrap());

/// Root Work 1..60: ten words against ten definitions each.
pub fn parse_root_work(lines: &[Line]) -> BTreeMap<u32, Part> {
    let mut out = BTreeMap::new();
    let mut cur: Option<(u32, Part)> = None;
    for l in lines {
        let t = l.text.as_str();
        if let Some(c) = ROOT_WORK.captures(t) {
            if let Some((n, p)) = cur.take() {
                out.insert(n, p);
            }
            let n: u32 = c[1].parse().unwrap();
            cur = Some((n, Part { name: format!("Root Work {n}"), ..Default::default() }));
            continue;
        }
        if t.starts_with("(Answers are on") || ROOT_REVIEW.is_match(t) || t.starts_with("ROOT ROUNDUP") || t.starts_with('•') {
            if let Some((n, p)) = cur.take() {
                out.insert(n, p);
            }
            continue;
        }
        if let Some((_, p)) = cur.as_mut() {
            if !t.starts_with("Match each") {
                match_line(t, &mut p.words, &mut p.choices);
            }
        }
    }
    if let Some((n, p)) = cur.take() {
        out.insert(n, p);
    }
    out
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct RootReview {
    pub range: String,
    pub roots: Part,
    pub bank: Vec<String>,
    pub fills: Vec<(u32, String)>,
    pub tf: Vec<(u32, String)>,
}

pub fn parse_root_reviews(lines: &[Line]) -> Vec<RootReview> {
    #[derive(Clone, Copy, PartialEq)]
    enum S {
        None,
        Match,
        Fill,
        Tf,
    }
    let mut out: Vec<RootReview> = Vec::new();
    let mut sec = S::None;
    let mut cur: Option<(u32, Vec<String>)> = None;
    let mut active = false;

    fn flush(out: &mut [RootReview], sec: S, cur: &mut Option<(u32, Vec<String>)>) {
        if let (Some((n, parts)), Some(r)) = (cur.take(), out.last_mut()) {
            let s = blanks(&join(parts.iter().map(String::as_str)));
            match sec {
                S::Fill => r.fills.push((n, s)),
                S::Tf => r.tf.push((n, s)),
                _ => {}
            }
        }
    }

    for l in lines {
        let t = l.text.as_str();
        if let Some(c) = ROOT_REVIEW.captures(t) {
            flush(&mut out, sec, &mut cur);
            out.push(RootReview { range: format!("{}\u{2013}{}", &c[1], &c[2]), roots: Part { name: "roots".into(), ..Default::default() }, ..Default::default() });
            sec = S::None;
            active = true;
            continue;
        }
        // The next Root Work / Root Roundup section also ends a review.
        if active && (ROOT_WORK.is_match(t) || (t.starts_with("ROOT ROUNDUP ") && !t.contains("REVIEW"))) {
            flush(&mut out, sec, &mut cur);
            active = false;
            sec = S::None;
            continue;
        }
        // One review's heading is missing from the book's text (46-50); its
        // block still opens with "Match It", so infer the range from the last one.
        if t == "Match It" && !active {
            let next = out.last().and_then(|r| r.range.split('\u{2013}').nth(1)).and_then(|b| b.parse::<u32>().ok()).map_or(1, |b| b + 1);
            out.push(RootReview { range: format!("{}\u{2013}{}", next, next + 4), roots: Part { name: "roots".into(), ..Default::default() }, ..Default::default() });
            active = true;
        }
        if !active {
            continue;
        }
        if t.starts_with("(Answers are on") {
            flush(&mut out, sec, &mut cur);
            active = false;
            continue;
        }
        match t {
            "Match It" => { flush(&mut out, sec, &mut cur); sec = S::Match; continue }
            "Fill-ins" => { flush(&mut out, sec, &mut cur); sec = S::Fill; continue }
            "True or False" => { flush(&mut out, sec, &mut cur); sec = S::Tf; continue }
            _ => {}
        }
        if t.starts_with("Match each") || t.starts_with("Fill in the blanks") || t.starts_with("If the statement") {
            continue;
        }
        let r = out.last_mut().unwrap();
        match sec {
            S::Match => {
                let (mut w, mut ch) = (Vec::new(), Vec::new());
                match_line(t, &mut w, &mut ch);
                r.roots.words.extend(w.into_iter().map(|(n, s)| (n, s.trim_end_matches(|c: char| c == '_' || c.is_whitespace()).to_string())));
                r.roots.choices.extend(ch);
            }
            S::Fill | S::Tf => {
                if let Some((n, rest)) = numbered(t) {
                    flush(&mut out, sec, &mut cur);
                    cur = Some((n, vec![rest]));
                } else if let Some((_, parts)) = cur.as_mut() {
                    parts.push(t.to_string());
                } else if sec == S::Fill {
                    // The word bank: columns separated by two or more spaces.
                    let r = out.last_mut().unwrap();
                    r.bank.extend(t.split("  ").map(str::trim).filter(|w| !w.is_empty()).map(String::from));
                }
            }
            S::None => {}
        }
    }
    flush(&mut out, sec, &mut cur);
    out
}

/// Key entries of one block: section name -> number -> value.
pub type SectionKey = BTreeMap<String, BTreeMap<u32, String>>;

/// "ROOT WORK n" lines in the key section: number -> (item -> letter).
pub fn root_work_keys(lines: &[Line]) -> BTreeMap<u32, BTreeMap<u32, String>> {
    let mut out: BTreeMap<u32, BTreeMap<u32, String>> = BTreeMap::new();
    let re = Regex::new(r"^ROOT WORK (\d+)$").unwrap();
    let mut cur = 0;
    for l in lines {
        if let Some(c) = re.captures(&l.text) {
            cur = c[1].parse().unwrap();
            out.entry(cur).or_default();
        } else if cur > 0 {
            for (n, v) in pairs(&l.text) {
                out.entry(cur).or_default().entry(n).or_insert(v);
            }
        }
    }
    out
}

/// "ROOT ROUNDUP REVIEW a–b" blocks in the key section.
pub fn root_review_keys(lines: &[Line]) -> BTreeMap<String, SectionKey> {
    let mut out: BTreeMap<String, SectionKey> = BTreeMap::new();
    let mut range = String::new();
    let mut sec = String::new();
    for l in lines {
        let t = l.text.as_str();
        if let Some(c) = ROOT_REVIEW.captures(t) {
            range = format!("{}\u{2013}{}", &c[1], &c[2]);
            sec.clear();
            out.entry(range.clone()).or_default();
        } else if matches!(t, "Match It" | "Fill-ins" | "True or False") {
            sec = t.to_string();
        } else if !range.is_empty() && !sec.is_empty() {
            for (n, v) in pairs(t) {
                out.get_mut(&range).unwrap().entry(sec.clone()).or_default().entry(n).or_insert(v);
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
    fn option_letters() {
        assert_eq!(option("Ⓒ abstemious").unwrap(), (2, "abstemious".into()));
        assert_eq!(option("🄵 model").unwrap(), (5, "model".into()));
        assert!(option("abstemious").is_none());
    }

    #[test]
    fn cumulative_sections() {
        let c = parse_cumulative(&lines(
            "
       Matching
       Match each word with its definition.
       Part A
       21. dapple                     (A) to mark with spots
       22. verdant                    (B) green with plants
       Sense or Nonsense
       31.\tThe lighthouse keeper counted the ships that drifted past. ____
       Fill-ins
       41.\t____________ the old harbor chart took the whole afternoon.
            Ⓐ Studying
            Ⓑ Ignoring
       46.\tThe committee wanted ____________ the rules and ____________ every deadline.
            Ⓐ respect for..ignored
            Ⓑ loyalty to..observed
",
        ));
        assert_eq!(c.parts.len(), 1);
        assert_eq!(c.parts[0].words, vec![(21, "dapple".into()), (22, "verdant".into())]);
        assert_eq!(c.sense.len(), 1);
        assert_eq!(c.mcq.len(), 2);
        assert_eq!(c.mcq[0].options, vec!["Studying", "Ignoring"]);
        assert_eq!(c.mcq[1].options[1], "loyalty to \u{2026} observed");
    }

    #[test]
    fn root_work_and_keys() {
        let w = parse_root_work(&lines(
            "
       Root Work 5
       Match each word with its definition.
        1. dapple                     (A) a measure of water depth
        2. fathom                     (B) to mark with spots of color
       (Answers are on page 411.)
",
        ));
        assert_eq!(w[&5].words.len(), 2);
        let k = root_work_keys(&lines("\n ROOT WORK 5\n 1. E  2. D  3. I               4. J\n"));
        assert_eq!(k[&5][&3], "I");
        assert_eq!(k[&5].len(), 4);
    }
}


// ---- assembling quizzes ----------------------------------------------------

/// A quiz ready to store: every item has its answer resolved.
#[derive(Debug, Clone)]
pub struct ExtraQuiz {
    pub id: String,
    pub title: String,
    pub kind: String,
    /// Catalog group: "reviews" or "roots".
    pub group: String,
    pub items: Vec<ExtraItem>,
}

#[derive(Debug, Clone)]
pub struct ExtraItem {
    pub ord: u32,
    /// Reading passage shown with the question (pretest / posttest).
    pub passage: Option<String>,
    /// How the app presents it: match, bank, mcq, sense, tf.
    pub style: &'static str,
    pub prompt: String,
    pub choices: Option<serde_json::Value>,
    pub answer: String,
    pub explanation: Option<String>,
    pub words: Vec<String>,
}

/// Word banks are printed in columns; two bank words can sit one space apart.
/// A chunk is split only when each part is a known answer, so "status quo" survives.
pub fn fix_bank(bank: &[String], answers: &[String]) -> Vec<String> {
    let known = |w: &str| answers.iter().any(|a| a.eq_ignore_ascii_case(w));
    bank.iter()
        .flat_map(|chunk| {
            let parts: Vec<&str> = chunk.split_whitespace().collect();
            if !known(chunk) && parts.len() > 1 && parts.iter().all(|p| known(p)) {
                parts.into_iter().map(String::from).collect()
            } else {
                vec![chunk.clone()]
            }
        })
        .collect()
}

fn matching_items(part: &Part, key: &BTreeMap<u32, String>, lemma: &dyn Fn(&str) -> String, errs: &mut Vec<String>, ctx: &str) -> Vec<ExtraItem> {
    let defs = serde_json::json!(part.choices.iter().map(|(_, d)| d).collect::<Vec<_>>());
    part.words
        .iter()
        .filter_map(|(n, w)| {
            let letter = key.get(n).and_then(|k| k.chars().next());
            let answer = letter.and_then(|l| part.answer(l));
            if answer.is_none() {
                errs.push(format!("{ctx}: no answer for item {n} ({w})"));
            }
            Some(ExtraItem { passage: None, ord: *n, style: "match", prompt: w.clone(), choices: Some(defs.clone()), answer: answer?.to_string(), explanation: None, words: vec![lemma(w)] })
        })
        .collect()
}

/// The two cumulative reviews as quizzes, joined to their keys.
pub fn cumulative_quiz(id: &str, title: &str, c: &Cumulative, key: &BTreeMap<u32, String>, lemma: &dyn Fn(&str) -> String, errs: &mut Vec<String>) -> ExtraQuiz {
    let mut items = Vec::new();
    for p in &c.parts {
        items.extend(matching_items(p, key, lemma, errs, &format!("{id} part {}", p.name)));
    }
    for (n, s) in &c.sense {
        match key.get(n).map(String::as_str) {
            Some("S") => items.push(ExtraItem { passage: None, ord: *n, style: "sense", prompt: s.clone(), choices: Some(serde_json::json!(["Sense", "Nonsense"])), answer: "Sense".into(), explanation: None, words: vec![] }),
            Some("N") => items.push(ExtraItem { passage: None, ord: *n, style: "sense", prompt: s.clone(), choices: Some(serde_json::json!(["Sense", "Nonsense"])), answer: "Nonsense".into(), explanation: None, words: vec![] }),
            other => errs.push(format!("{id}: sense item {n} has key {other:?}")),
        }
    }
    for m in &c.mcq {
        let idx = key.get(&m.num).and_then(|k| k.chars().next()).and_then(|c| (c as u32).checked_sub('A' as u32)).map(|i| i as usize);
        match idx.and_then(|i| m.options.get(i)) {
            Some(a) => items.push(ExtraItem { passage: None, ord: m.num, style: "mcq", prompt: m.stem.clone(), choices: Some(serde_json::json!(m.options)), answer: a.clone(), explanation: None, words: vec![] }),
            None => errs.push(format!("{id}: question {} has key {:?} for {} options", m.num, key.get(&m.num), m.options.len())),
        }
    }
    items.sort_by_key(|i| i.ord);
    ExtraQuiz { id: id.into(), title: title.into(), kind: "review".into(), group: "reviews".into(), items }
}

pub fn root_work_quiz(n: u32, part: &Part, key: &BTreeMap<u32, String>, lemma: &dyn Fn(&str) -> String, errs: &mut Vec<String>) -> ExtraQuiz {
    ExtraQuiz {
        id: format!("rw-{n:02}"),
        title: format!("Root Work {n}"),
        kind: "matching".into(),
        group: "roots".into(),
        items: matching_items(part, key, lemma, errs, &format!("root work {n}")),
    }
}

pub fn root_review_quiz(r: &RootReview, key: &SectionKey, lemma: &dyn Fn(&str) -> String, errs: &mut Vec<String>) -> ExtraQuiz {
    let id = format!("rr-{}", r.range.replace('\u{2013}', "-"));
    let mut items = Vec::new();
    // Roots against meanings.
    let mm = key.get("Match It").cloned().unwrap_or_default();
    for it in matching_items(&r.roots, &mm, lemma, errs, &format!("{id} match")) {
        items.push(ExtraItem { passage: None, ord: it.ord, words: vec![], ..it });
    }
    // Definitions against a word bank.
    let fk = key.get("Fill-ins").cloned().unwrap_or_default();
    let answers: Vec<String> = fk.values().cloned().collect();
    let bank = serde_json::json!(fix_bank(&r.bank, &answers));
    for (n, prompt) in &r.fills {
        match fk.get(n) {
            Some(a) => items.push(ExtraItem { passage: None, ord: 100 + n, style: "bank", prompt: prompt.clone(), choices: Some(bank.clone()), answer: a.clone(), explanation: None, words: vec![lemma(a)] }),
            None => errs.push(format!("{id}: fill-in {n} has no answer")),
        }
    }
    // True or false.
    let tk = key.get("True or False").cloned().unwrap_or_default();
    for (n, s) in &r.tf {
        match tk.get(n).map(String::as_str) {
            Some("T") => items.push(ExtraItem { passage: None, ord: 200 + n, style: "tf", prompt: s.clone(), choices: Some(serde_json::json!(["True", "False"])), answer: "True".into(), explanation: None, words: vec![] }),
            Some("F") => items.push(ExtraItem { passage: None, ord: 200 + n, style: "tf", prompt: s.clone(), choices: Some(serde_json::json!(["True", "False"])), answer: "False".into(), explanation: None, words: vec![] }),
            other => errs.push(format!("{id}: true/false {n} has key {other:?}")),
        }
    }
    ExtraQuiz { id, title: format!("Root Roundup review {}", r.range), kind: "review".into(), group: "roots".into(), items }
}
