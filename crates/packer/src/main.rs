mod barrons;
mod cmudict;
mod gcide;
mod moby;
mod oald;
mod oald_audio;
mod wiktionary;
mod wordnet;

use std::path::PathBuf;
use std::process::Command;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(about = "Builds dictionary packs and imports Barron's for the GRE app")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Build the pronunciation fallback pack from cmudict.dict.
    Cmudict { input: PathBuf, out: PathBuf },
    /// Build the Webster 1913 pack from the extracted GCIDE directory (CIDE.* files).
    Gcide { dir: PathBuf, out: PathBuf },
    /// Build the typo-search index (`wiktionary.fst`) from a pack.
    Fst { pack: PathBuf, out: PathBuf },
    /// Build the WordNet pack from english-wordnet-*.xml.gz.
    Wordnet { input: PathBuf, out: PathBuf },
    /// Build the Moby thesaurus pack from mthesaur.txt.
    Moby { input: PathBuf, out: PathBuf },
    /// Build the OALD pack from the extracted OALData directory.
    Oald {
        /// Path to the OALData directory (contains json/, sounds/, …).
        dir: PathBuf,
        out: PathBuf,
    },
    /// Build the OALD recordings pack (`oald-audio.pack`) from the MP3s build_oald_audio.py wrote.
    OaldAudio {
        /// Path to the OALData directory (contains json/oald_audio.json and audio/).
        dir: PathBuf,
        out: PathBuf,
    },
    /// Print an entry (or key-prefix matches) from a pack, for inspection.
    Lookup {
        pack: PathBuf,
        word: String,
        /// List headwords starting with this word instead of printing the entry.
        #[arg(long)]
        prefix: bool,
    },
    /// Build the Wiktionary pack from the kaikki.org JSONL(.gz) dump.
    Wiktionary {
        input: PathBuf,
        out: PathBuf,
        /// Stop after this many input lines (for quick experiments).
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Parse the Barron's PDF and print per-unit statistics (no files written yet).
    Barrons {
        pdf: PathBuf,
        /// Write the parsed book to this SQLite file (keep it under private/).
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

fn pdf_text(pdf: &PathBuf) -> Result<String> {
    let out = Command::new("pdftotext")
        .args(["-layout"])
        .arg(pdf)
        .arg("-")
        .output()
        .context("running pdftotext (install poppler-utils)")?;
    if !out.status.success() {
        bail!("pdftotext failed");
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Cmudict { input, out } => cmudict::build(&input, &out)?,
        Cmd::Gcide { dir, out } => gcide::build(&dir, &out)?,
        Cmd::Fst { pack, out } => println!("{} keys", gredict_core::fuzzy::Fuzzy::build(&pack, &out)?),
        Cmd::Wordnet { input, out } => wordnet::build(&input, &out)?,
        Cmd::Moby { input, out } => moby::build(&input, &out)?,
        Cmd::Oald { dir, out } => oald::build(&dir, &out)?,
        Cmd::OaldAudio { dir, out } => oald_audio::build(&dir, &out)?,
        Cmd::Lookup { pack, word, prefix } => {
            let p = gredict_core::pack::Pack::open(&pack)?;
            let key = gredict_core::lemma::key(&word);
            if prefix {
                for w in p.prefix(&key, 50)? {
                    println!("{w}");
                }
            } else {
                match p.get(&key)? {
                    Some(e) => println!("{}", serde_json::to_string_pretty(&e)?),
                    None => println!("not found: {key}"),
                }
            }
        }
        Cmd::Wiktionary { input, out, limit } => wiktionary::build(&input, &out, limit)?,
        Cmd::Barrons { pdf, out } => {
            let lines = barrons::text::lines(&pdf_text(&pdf)?);
            let units = barrons::split_units(&lines);
            let keys = barrons::exercises::parse_keys(barrons::key_section(&lines));
            let mut words = 0;
            let mut problems = Vec::new();
            let mut all_entries = Vec::new();
            let mut all_reviews = Vec::new();
            let mut extra_quizzes: Vec<barrons::extras::ExtraQuiz> = Vec::new();
            let mut all_roots: Vec<barrons::extras::Root> = Vec::new();
            for (n, body, review) in &units {
                let es = barrons::entries::parse(*n, body);
                words += es.len();
                // Review lines run to the next unit; unit 80's run to the end
                // of the book, so cut at the next section heading.
                let cut = review
                    .iter()
                    .position(|l| l.text.starts_with("REVIEW:") || l.text.starts_with("300 HIGH"))
                    .unwrap_or(review.len());
                let r = barrons::exercises::parse_review(*n, &review[..cut]);
                for q in &r.applying {
                    if q.answers.is_empty() || q.blanks.iter().any(|b| b.len() < 3) {
                        problems.push(format!("unit {n} applying Q{}: answers={:?} raw={:?} blanks={:?}", q.number, q.answers, q.raw_answer, q.blanks));
                    }
                }
                let k = keys.get(n);
                let shape = (r.matching_words.len(), r.matching_choices.len(), r.bank.len(), r.fill_ins.len(), r.sense.len());
                if es.len() != 10 || shape != (10, 10, 10, 10, 5) {
                    problems.push(format!("unit {n}: entries={} (match words, choices, bank, fill, sense)={shape:?}", es.len()));
                }
                match k {
                    Some(k) if (k.matching.len(), k.fill_ins.len(), k.sense.len()) == (10, 10, 5) => {
                        // Every key answer must fit its exercise.
                        let bank: std::collections::HashSet<_> = r.bank.iter().map(|w| w.to_lowercase()).collect();
                        for (i, a) in &k.fill_ins {
                            if !bank.contains(&a.to_lowercase()) {
                                problems.push(format!("unit {n}: fill-in {i} answer {a:?} not in word bank"));
                            }
                        }
                        if k.matching.iter().any(|(_, c)| !('A'..='J').contains(c)) || k.sense.iter().any(|(_, c)| !matches!(c, 'S' | 'N')) {
                            problems.push(format!("unit {n}: bad key letters"));
                        }
                    }
                    other => problems.push(format!("unit {n}: key={:?}", other.map(|k| (k.matching.len(), k.fill_ins.len(), k.sense.len())))),
                }
                if let Some(k) = keys.get(n) {
                    all_reviews.push((r, barrons::exercises::Key { matching: k.matching.clone(), fill_ins: k.fill_ins.clone(), sense: k.sense.clone() }));
                }
                all_entries.extend(es);
            }
            println!("{} units, {words} entries", units.len());
            match barrons::find_regions(&lines) {
                None => problems.push("could not locate the reviews / roots sections".to_string()),
                Some(r) => {
                    use barrons::extras::*;
                    for (name, region, key_region) in [("high-frequency review", r.hf, r.hf_key), ("essential review", r.ew, r.ew_key)] {
                        let c = parse_cumulative(region);
                        let k = flat_key(key_region);
                        let items = c.parts.iter().map(|p| p.words.len()).sum::<usize>() + c.sense.len() + c.mcq.len();
                        println!("{name}: {} parts ({} words), {} sense, {} mcq, key {}", c.parts.len(), c.parts.iter().map(|p| p.words.len()).sum::<usize>(), c.sense.len(), c.mcq.len(), k.len());
                        if items != 50 || k.len() != 50 {
                            problems.push(format!("{name}: items={items} key={}", k.len()));
                        }
                    }
                    let roots = parse_roots(r.roots);
                    let rw = parse_root_work(r.roots);
                    let rwk = root_work_keys(r.root_work_keys);
                    let rr = parse_root_reviews(r.roots);
                    let rrk = root_review_keys(r.root_review_keys);
                    println!("roots: {} (words {}), root work {} (keys {}), root reviews {} (keys {})", roots.len(), roots.iter().map(|x| x.words.len()).sum::<usize>(), rw.len(), rwk.len(), rr.len(), rrk.len());
                    for (n, p) in &rw {
                        if p.words.len() != 10 || p.choices.len() != 10 || rwk.get(n).map_or(0, |k| k.len()) != 10 {
                            problems.push(format!("root work {n}: words {} choices {} key {}", p.words.len(), p.choices.len(), rwk.get(n).map_or(0, |k| k.len())));
                        }
                    }
                    // Assemble every quiz with its answers resolved.
                    let lem = |w: &str| gredict_core::lemma::key(w);
                    let mut errs = Vec::new();
                    extra_quizzes.push(cumulative_quiz("hf-review", "Review: 300 high-frequency words", &parse_cumulative(r.hf), &flat_key(r.hf_key), &lem, &mut errs));
                    extra_quizzes.push(cumulative_quiz("ew-review", "Review: Essential Words for the GRE", &parse_cumulative(r.ew), &flat_key(r.ew_key), &lem, &mut errs));
                    for (n, p) in &rw {
                        if let Some(k) = rwk.get(n) {
                            extra_quizzes.push(root_work_quiz(*n, p, k, &lem, &mut errs));
                        }
                    }
                    for rv in &rr {
                        if let Some(k) = rrk.get(&rv.range) {
                            extra_quizzes.push(root_review_quiz(rv, k, &lem, &mut errs));
                        }
                    }
                    for (id, title, region, key_region) in [("pretest", "Pretest", r.pretest, r.pretest_key), ("posttest", "Posttest", r.posttest, r.posttest_key)] {
                        let raws = barrons::gre_test::parse_questions(region);
                        let key = barrons::gre_test::parse_key(key_region);
                        let q = barrons::gre_test::quiz(id, title, &raws, &key, &mut errs);
                        println!("{id}: {} questions parsed, {} in key, {} resolved", raws.len(), key.len(), q.items.len());
                        if raws.len() != 20 || key.len() != 20 || q.items.len() != 20 {
                            errs.push(format!("{id}: expected 20 resolved questions"));
                        }
                        extra_quizzes.push(q);
                    }
                    problems.extend(errs);
                    all_roots = roots;
                    for rv in &rr {
                        let k = rrk.get(&rv.range);
                        let answers: Vec<String> = k.and_then(|k| k.get("Fill-ins")).map(|m| m.values().cloned().collect()).unwrap_or_default();
                        let bank = fix_bank(&rv.bank, &answers);
                        let sizes = (rv.roots.words.len(), rv.roots.choices.len(), bank.len(), rv.fills.len(), rv.tf.len());
                        if sizes != (10, 10, 10, 10, 5) || k.is_none_or(|k| (k.get("Match It").map_or(0, |m| m.len()), k.get("Fill-ins").map_or(0, |m| m.len()), k.get("True or False").map_or(0, |m| m.len())) != (10, 10, 5)) {
                            problems.push(format!("root review {}: {sizes:?} key {:?}", rv.range, k.map(|k| k.iter().map(|(a, b)| (a.clone(), b.len())).collect::<Vec<_>>())));
                        }
                    }
                }
            }
            for p in &problems {
                println!("PROBLEM {p}");
            }
            if let Some(out) = out {
                if !problems.is_empty() {
                    bail!("refusing to write {} with {} problems", out.display(), problems.len());
                }
                barrons::store::write(&out, &all_entries, &all_reviews, &extra_quizzes, &all_roots)?;
                println!("wrote {} ({} extra quizzes, {} roots)", out.display(), extra_quizzes.len(), all_roots.len());
            }
        }
    }
    Ok(())
}
