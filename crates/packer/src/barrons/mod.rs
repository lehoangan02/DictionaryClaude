//! Importer for Barron's *Essential Words for the GRE* (4th ed.), parsed from
//! `pdftotext -layout` output. Output stays under `private/`: the content is
//! copyrighted and must never be committed or shipped.

pub mod entries;
pub mod exercises;
pub mod extras;
pub mod gre_test;
pub mod store;
pub mod text;

use std::sync::LazyLock;

use regex::Regex;

use text::Line;

static UNIT_HEAD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^UNIT (\d{1,2})$").unwrap());
static REVIEW_HEAD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^REVIEW (\d{1,2})$").unwrap());

/// Splits the vocabulary section into per-unit (entry lines, review lines).
/// Entry lines run from `UNIT n` to `REVIEW n`; review lines run from
/// `REVIEW n` to the next `UNIT`.
pub fn split_units(lines: &[Line]) -> Vec<(u32, Vec<Line>, Vec<Line>)> {
    let mut out: Vec<(u32, Vec<Line>, Vec<Line>)> = Vec::new();
    let mut in_review = false;
    for l in lines {
        if let Some(c) = UNIT_HEAD.captures(&l.text) {
            let n: u32 = c[1].parse().unwrap();
            // The answer keys later in the book repeat `UNIT 1..80`; stop
            // when numbering restarts.
            if out.last().is_some_and(|(prev, ..)| n <= *prev) {
                break;
            }
            out.push((n, Vec::new(), Vec::new()));
            in_review = false;
        } else if REVIEW_HEAD.is_match(&l.text) && !out.is_empty() {
            in_review = true;
        } else if let Some(u) = out.last_mut() {
            if in_review { &mut u.2 } else { &mut u.1 }.push(l.clone());
        }
    }
    out
}

/// The slice of `lines` that holds the per-unit answer keys: it starts at the
/// first `UNIT n` heading where the numbering restarts after the vocabulary.
pub fn key_section(lines: &[Line]) -> &[Line] {
    let mut prev = 0u32;
    for (i, l) in lines.iter().enumerate() {
        if let Some(c) = UNIT_HEAD.captures(&l.text) {
            let n: u32 = c[1].parse().unwrap();
            if n <= prev {
                return &lines[i..];
            }
            prev = n;
        }
    }
    &[]
}

#[cfg(test)]
mod golden {
    //! Runs against the owner's local copy of the book (`private/`, never
    //! committed); silently skipped when it is absent.
    use super::*;
    use crate::barrons::{entries, exercises, text};

    fn book() -> Option<Vec<Line>> {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../private/barrons.txt");
        Some(text::lines(&std::fs::read_to_string(p).ok()?))
    }

    #[test]
    fn whole_book_has_the_expected_shape() {
        let Some(lines) = book() else { return };
        let units = split_units(&lines);
        assert_eq!(units.len(), 80);
        let keys = exercises::parse_keys(key_section(&lines));
        assert_eq!(keys.len(), 80);
        for (n, body, review) in &units {
            assert_eq!(entries::parse(*n, body).len(), 10, "unit {n} entries");
            let r = exercises::parse_review(*n, review);
            assert_eq!(
                (r.matching_words.len(), r.matching_choices.len(), r.bank.len(), r.fill_ins.len(), r.sense.len()),
                (10, 10, 10, 10, 5),
                "unit {n} review shape"
            );
            let k = &keys[n];
            assert_eq!((k.matching.len(), k.fill_ins.len(), k.sense.len()), (10, 10, 5), "unit {n} key");
            if n % 10 == 0 {
                assert!(r.applying.iter().all(|q| !q.answers.is_empty()), "unit {n} applying answers");
            }
        }
    }

    #[test]
    fn unit_6_key_and_unit_1_first_entries() {
        let Some(lines) = book() else { return };
        let keys = exercises::parse_keys(key_section(&lines));
        let k6 = &keys[&6];
        // Ten distinct letters A-J: a permutation, as a matching key must be.
        let mut letters: Vec<char> = k6.matching.iter().map(|(_, c)| *c).collect();
        letters.sort();
        assert_eq!(letters.iter().collect::<String>(), "ABCDEFGHIJ");
        assert!(k6.fill_ins.iter().all(|(_, w)| !w.is_empty() && w.chars().all(|c| c.is_alphabetic())));
        let units = split_units(&lines);
        let es = entries::parse(1, &units[0].1);
        assert_eq!(es[0].headword, "abate");
        assert_eq!(es[1].headword, "abdicate");
        assert!(es[2].high_freq && es[2].headword == "aberrant");
    }

    #[test]
    fn reading_questions_carry_their_whole_passage() {
        let Some(lines) = book() else { return };
        let r = find_regions(&lines).expect("regions");
        for (name, region) in [("pretest", r.pretest), ("posttest", r.posttest)] {
            let qs = gre_test::parse_questions(region);
            assert_eq!(qs.len(), 20, "{name}");
            for q in qs.iter().filter(|q| q.num >= 15) {
                let p = q.passage.as_deref().unwrap_or_else(|| panic!("{name} Q{} has no passage", q.num));
                assert!(p.lines().any(|l| l.trim_start().starts_with("(5)")), "{name} Q{} passage has line numbers:\n{p}", q.num);
            }
        }
        let pre = gre_test::parse_questions(r.pretest);
        let p = pre[19].passage.as_deref().unwrap();
        assert!(p.lines().count() > 40, "Q20 sees the whole passage, not just its end: {} lines", p.lines().count());
    }

}

/// Where the parts outside the 80 units sit in the cleaned line stream.
pub struct Regions<'a> {
    pub hf: &'a [Line],
    pub hf_key: &'a [Line],
    pub ew: &'a [Line],
    pub ew_key: &'a [Line],
    /// Root lists, Root Work and Root Roundup reviews (everything up to the suffixes).
    pub roots: &'a [Line],
    /// The answer-key section, from the first `UNIT 1` key onwards.
    pub keys: &'a [Line],
    pub root_work_keys: &'a [Line],
    pub root_review_keys: &'a [Line],
    pub pretest: &'a [Line],
    pub pretest_key: &'a [Line],
    pub posttest: &'a [Line],
    pub posttest_key: &'a [Line],
}

pub fn find_regions(lines: &[Line]) -> Option<Regions<'_>> {
    let pos = |from: usize, f: &dyn Fn(&Line) -> bool| lines[from..].iter().position(f).map(|i| i + from);
    let apply80 = pos(0, &|l| l.text.starts_with("APPLYING YOUR KNOWLEDGE") && l.text.contains("71"))?;
    let hf = pos(apply80, &|l| l.text == "Matching")?;
    let hf_ans = pos(hf, &|l| l.text == "REVIEW: 300 HIGH-FREQUENCY GRE WORDS ANSWERS")?;
    let ew_title = pos(hf_ans, &|l| l.text == "Essential Words for the GRE")?;
    let ew_ans = pos(ew_title, &|l| l.text == "REVIEW: ESSENTIAL WORDS FOR THE GRE ANSWERS")?;
    let roots_start = pos(ew_ans, &|l| l.text.starts_with('•'))?;
    let suffixes = pos(roots_start, &|l| l.text.eq_ignore_ascii_case("Common Suffixes"))?;
    let keys_start = lines.len() - key_section(lines).len();
    let rr = pos(keys_start, &|l| l.text == "ROOT ROUNDUP")?;
    let rrr = pos(rr, &|l| l.text == "ROOT ROUNDUP REVIEW")?;
    let pre = pos(0, &|l| l.text == "Pretest")?;
    let pre_ans = pos(pre, &|l| l.text == "PRETEST ANSWERS")?;
    let pre_end = pos(pre_ans, &|l| l.text.starts_with("YOUR PRETEST SCORE")).unwrap_or(pre_ans + 25);
    let post = pos(suffixes, &|l| l.text == "Posttest")?;
    let post_ans = pos(post, &|l| l.text == "POSTTEST ANSWERS")?;
    let post_end = pos(post_ans, &|l| l.text.starts_with("YOUR POSTTEST SCORE")).unwrap_or(post_ans + 25);
    Some(Regions {
        hf: &lines[hf..hf_ans],
        hf_key: &lines[hf_ans..ew_title.min(hf_ans + 40)],
        ew: &lines[ew_title..ew_ans],
        ew_key: &lines[ew_ans..(ew_ans + 30).min(roots_start)],
        roots: &lines[roots_start..suffixes],
        keys: &lines[keys_start..],
        root_work_keys: &lines[rr..rrr],
        root_review_keys: &lines[rrr..],
        pretest: &lines[pre..pre_ans],
        pretest_key: &lines[pre_ans + 1..pre_end],
        posttest: &lines[post..post_ans],
        posttest_key: &lines[post_ans + 1..post_end],
    })
}
