//! The tutor-note grammar and the relation graph built from it.
//!
//! ```text
//! abate = lessen = subside         same meaning
//! frugal ≈ thrifty ≈ sparing       roughly similar
//! lucid >< opaque                  opposite
//! imbibition ⍯ osmosis             different / easily confused
//! ```
//! A side may list several terms separated by commas, and chains are allowed.
//! ASCII fallbacks for phone keyboards: `~=` for ≈, `!=` or `/=` for ⍯/≠,
//! `<>` or `> <` for `><`. Lines without an operator are free text.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};

use crate::lemma;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rel {
    Same,
    Similar,
    Opposite,
    Different,
}

/// Longest operators first so `> <` is not read as two stray characters.
const OPS: &[(&str, Rel)] = &[
    ("> <", Rel::Opposite),
    ("><", Rel::Opposite),
    ("<>", Rel::Opposite),
    ("~=", Rel::Similar),
    ("≈", Rel::Similar),
    ("!=", Rel::Different),
    ("/=", Rel::Different),
    ("≠", Rel::Different),
    ("⍯", Rel::Different),
    ("=", Rel::Same),
];

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Edge {
    pub a: String,
    pub b: String,
    pub rel: Rel,
}

/// A term as the user wrote it, plus its lemma key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Term {
    pub text: String,
    pub key: String,
}

fn term(raw: &str) -> Option<Term> {
    // Drop a trailing parenthetical gloss such as "abate (v.)".
    let base = raw.split('(').next().unwrap_or(raw);
    let text = base.trim().trim_matches(|c: char| matches!(c, '.' | ';' | ':' | '"' | '“' | '”')).trim();
    if text.is_empty() || text.chars().count() > 40 {
        return None;
    }
    Some(Term { text: text.to_string(), key: lemma::key(text) })
}

fn side(raw: &str) -> Vec<Term> {
    raw.split(',').filter_map(term).collect()
}

/// Splits a line into alternating sides and operators.
fn split(line: &str) -> (Vec<&str>, Vec<Rel>) {
    let (mut sides, mut ops) = (Vec::new(), Vec::new());
    let (mut start, mut i) = (0, 0);
    'scan: while i < line.len() {
        for (sym, rel) in OPS {
            if line[i..].starts_with(sym) {
                sides.push(&line[start..i]);
                ops.push(*rel);
                i += sym.len();
                start = i;
                continue 'scan;
            }
        }
        i += line[i..].chars().next().map_or(1, char::len_utf8);
    }
    sides.push(&line[start..]);
    (sides, ops)
}

pub fn parse_line(line: &str) -> Vec<Edge> {
    let (sides, ops) = split(line);
    if ops.is_empty() {
        return Vec::new();
    }
    let sides: Vec<Vec<Term>> = sides.into_iter().map(side).collect();
    let mut edges = BTreeSet::new();
    let mut link = |x: &Term, y: &Term, rel: Rel| {
        if x.key != y.key {
            let (a, b) = if x.key <= y.key { (&x.key, &y.key) } else { (&y.key, &x.key) };
            edges.insert(Edge { a: a.clone(), b: b.clone(), rel });
        }
    };
    // Adjacent sides are related by their operator.
    for (i, rel) in ops.iter().enumerate() {
        for x in &sides[i] {
            for y in &sides[i + 1] {
                link(x, y, *rel);
            }
        }
    }
    // Terms listed together on one side share a meaning in groups of
    // Same/Similar ("a, b = c" means a and b are alike too).
    for (i, group) in sides.iter().enumerate() {
        let rel = [i.checked_sub(1).map(|j| ops[j]), ops.get(i).copied()]
            .into_iter()
            .flatten()
            .find(|r| matches!(r, Rel::Same | Rel::Similar));
        if let Some(rel) = rel {
            for (n, x) in group.iter().enumerate() {
                for y in &group[n + 1..] {
                    link(x, y, rel);
                }
            }
        }
    }
    // A chain of one operator ("a = b = c") relates every pair in it.
    let mut i = 0;
    while i < ops.len() {
        let mut j = i;
        while j + 1 < ops.len() && ops[j + 1] == ops[i] {
            j += 1;
        }
        if j > i && matches!(ops[i], Rel::Same | Rel::Similar) {
            let members: Vec<&Term> = sides[i..=j + 1].iter().flatten().collect();
            for (n, x) in members.iter().enumerate() {
                for y in &members[n + 1..] {
                    link(x, y, ops[i]);
                }
            }
        }
        i = j + 1;
    }
    edges.into_iter().collect()
}

/// Edges for a whole note body (one relation per line).
pub fn parse(body: &str) -> Vec<Edge> {
    body.lines().flat_map(parse_line).collect::<BTreeSet<_>>().into_iter().collect()
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct Related {
    pub same: Vec<String>,
    pub similar: Vec<String>,
    pub opposite: Vec<String>,
    pub different: Vec<String>,
}

impl Related {
    pub fn is_empty(&self) -> bool {
        self.same.is_empty() && self.similar.is_empty() && self.opposite.is_empty() && self.different.is_empty()
    }
}

#[derive(Debug, Default)]
pub struct Graph {
    adj: HashMap<String, Vec<(String, Rel)>>,
}

impl Graph {
    pub fn new(edges: impl IntoIterator<Item = Edge>) -> Self {
        let mut adj: HashMap<String, Vec<(String, Rel)>> = HashMap::new();
        for e in edges {
            adj.entry(e.a.clone()).or_default().push((e.b.clone(), e.rel));
            adj.entry(e.b).or_default().push((e.a, e.rel));
        }
        Graph { adj }
    }

    fn component(&self, start: &str, allow: &[Rel]) -> HashSet<String> {
        let mut seen = HashSet::from([start.to_string()]);
        let mut q = VecDeque::from([start.to_string()]);
        while let Some(n) = q.pop_front() {
            for (m, r) in self.adj.get(&n).into_iter().flatten() {
                if allow.contains(r) && seen.insert(m.clone()) {
                    q.push_back(m.clone());
                }
            }
        }
        seen
    }

    /// Everything the notes say about `key`, with closure: synonyms of
    /// synonyms are synonyms, and the opposite of a synonym is an opposite.
    pub fn related(&self, key: &str) -> Related {
        let same = self.component(key, &[Rel::Same]);
        let loose = self.component(key, &[Rel::Same, Rel::Similar]);
        let mut opposite = HashSet::new();
        let mut different = HashSet::new();
        for s in &same {
            for (m, r) in self.adj.get(s).into_iter().flatten() {
                match r {
                    Rel::Opposite => opposite.extend(self.component(m, &[Rel::Same])),
                    Rel::Different => {
                        different.insert(m.clone());
                    }
                    _ => {}
                }
            }
        }
        let sorted = |set: HashSet<String>, drop: &[&HashSet<String>]| {
            let mut v: Vec<String> = set.into_iter().filter(|k| k != key && !drop.iter().any(|d| d.contains(k))).collect();
            v.sort();
            v
        };
        let empty = HashSet::new();
        Related {
            similar: sorted(loose.clone(), &[&same]),
            same: sorted(same.clone(), &[&empty]),
            opposite: sorted(opposite, &[&same]),
            different: sorted(different, &[&same]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(line: &str) -> Vec<(String, String, Rel)> {
        parse_line(line).into_iter().map(|e| (e.a, e.b, e.rel)).collect()
    }

    #[test]
    fn chain_of_synonyms_relates_every_pair() {
        let e = pairs("abate = lessen = subside");
        assert_eq!(e.len(), 3);
        assert!(e.contains(&("abate".into(), "subside".into(), Rel::Same)));
        assert!(e.iter().all(|(_, _, r)| *r == Rel::Same));
    }

    #[test]
    fn all_four_symbols_and_ascii_fallbacks() {
        assert_eq!(pairs("frugal ≈ thrifty"), pairs("frugal ~= thrifty"));
        assert_eq!(pairs("lucid >< opaque")[0].2, Rel::Opposite);
        assert_eq!(pairs("lucid <> opaque"), pairs("lucid > < opaque".replace("> <", "> <").as_str()));
        assert_eq!(pairs("imbibition ⍯ osmosis")[0].2, Rel::Different);
        assert_eq!(pairs("imbibition ≠ osmosis"), pairs("imbibition != osmosis"));
    }

    #[test]
    fn free_text_and_glosses() {
        assert!(pairs("Tutor said to review roots on Friday").is_empty());
        let e = pairs("abate (v.) = lessen.");
        assert_eq!(e, vec![("abate".into(), "lessen".into(), Rel::Same)]);
    }

    #[test]
    fn comma_groups_and_mixed_operators() {
        let e = pairs("abate, wane = lessen >< intensify");
        assert!(e.contains(&("abate".into(), "wane".into(), Rel::Same)));
        assert!(e.contains(&("lessen".into(), "wane".into(), Rel::Same)));
        assert!(e.contains(&("intensify".into(), "lessen".into(), Rel::Opposite)));
        // "abate" is not directly tied to the antonym of its neighbour.
        assert!(!e.iter().any(|(a, b, _)| (a == "abate" && b == "intensify") || (b == "abate" && a == "intensify")));
    }

    #[test]
    fn closure_in_the_graph() {
        let g = Graph::new(parse("abate = lessen\nlessen = subside\nabate >< intensify\nintensify = heighten\nabate ≈ ebb\nabate ⍯ abdicate"));
        let r = g.related("subside");
        assert_eq!(r.same, vec!["abate", "lessen"]);
        assert_eq!(r.opposite, vec!["heighten", "intensify"]);
        assert_eq!(r.similar, vec!["ebb"]);
        assert_eq!(r.different, vec!["abdicate"]);
        assert!(g.related("unknown").is_empty());
    }
}
