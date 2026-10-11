//! Webster's 1913 dictionary as distributed in GCIDE (public domain) -> pack.
//!
//! Entries start at a paragraph holding `<ent>`; their definitions are `<def>`
//! tags and the quotations in the paragraphs that follow belong to the last
//! definition. Entities use an unterminated form (`<adot/`) that must be
//! replaced *before* tags are stripped, or a tag would swallow the text up to
//! the next `>`.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::LazyLock;

use anyhow::{bail, Result};
use gredict_core::entry::{Block, Entry, Example, Sense};
use gredict_core::{lemma, pack};
use regex::Regex;
use rusqlite::{params, Connection};

static ENTITY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<([A-Za-z]+[0-9]*|\?)/").unwrap());
static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"</?[A-Za-z][A-Za-z0-9]*[^>]*>").unwrap());
static DEF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<def>(.*?)</def>").unwrap());
static ENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<ent>(.*?)</ent>").unwrap());
static POS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<pos>(.*?)</pos>").unwrap());
static ETY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<ety>(.*?)</ety>").unwrap());
static AS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<as>(.*?)</as>").unwrap());
static QUOTE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<q>(.*?)</q>").unwrap());
static QAU: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<qau>(.*?)</qau>").unwrap());
static MARK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<(?:mark|fld)>(.*?)</(?:mark|fld)>").unwrap());

fn entity(name: &str) -> &'static str {
    match name {
        "ae" => "æ", "AE" => "Æ", "oe" => "œ", "OE" => "Œ",
        "eacute" => "é", "egrave" => "è", "ecirc" => "ê", "euml" => "ë",
        "aacute" => "á", "agrave" => "à", "acirc" => "â", "auml" => "ä", "aring" => "å",
        "iacute" => "í", "icirc" => "î", "iuml" => "ï", "oacute" => "ó", "ocirc" => "ô",
        "ouml" => "ö", "uacute" => "ú", "ucirc" => "û", "uuml" => "ü", "ugrave" => "ù",
        "ccedil" => "ç", "ntilde" => "ñ", "Eacute" => "É", "deg" => "°",
        "frac12" => "½", "frac14" => "¼", "frac34" => "¾", "times" => "×",
        "lsquo" => "‘", "rsquo" => "’", "ldquo" => "“", "rdquo" => "”", "mdash" => "—", "ndash" => "–",
        _ => "",
    }
}

/// Webster capitalizes every headword; ordinary words read better lowercase.
/// Words with other capitals inside (proper names, "McCoy") keep theirs.
fn display(h: &str) -> String {
    let mut cs = h.chars();
    match cs.next() {
        Some(f) if f.is_uppercase() && cs.all(|c| !c.is_uppercase()) => h.to_lowercase(),
        _ => h.to_string(),
    }
}

/// Plain text of a markup fragment.
pub fn clean(s: &str) -> String {
    let s = ENTITY.replace_all(s, |c: &regex::Captures| entity(&c[1]).to_string());
    let s = TAG.replace_all(&s, "");
    s.replace("&amp;", "&").split_whitespace().collect::<Vec<_>>().join(" ")
}

fn pos_name(raw: &str) -> &'static str {
    let r = raw.trim().to_lowercase();
    let first = r.split([' ', ',']).next().unwrap_or("");
    match first {
        "v." | "v" => "verb",
        "a." | "a" => "adj",
        "adv." => "adv",
        "n." | "n" => "noun",
        "prep." => "prep",
        "conj." => "conj",
        "interj." => "intj",
        "pron." => "pron",
        "p." | "imp." | "pl." => "other",
        _ => "other",
    }
}

#[derive(Default)]
struct Raw {
    word: String,
    pos: String,
    etymology: String,
    senses: Vec<Sense>,
}

fn flush(raw: Option<Raw>, out: &mut BTreeMap<String, Entry>) {
    let Some(r) = raw else { return };
    if r.senses.is_empty() || r.word.is_empty() {
        return;
    }
    let key = lemma::key(&r.word);
    let mut block = Block { pos: pos_name(&r.pos).to_string(), senses: r.senses, ..Default::default() };
    if !r.etymology.is_empty() {
        block.etymology = Some(r.etymology);
    }
    let e = out.entry(key).or_insert_with(|| Entry { word: r.word.clone(), blocks: Vec::new() });
    if e.word != r.word {
        block.word = Some(r.word);
    }
    e.blocks.push(block);
}

/// Parses the text of one `CIDE.*` file into entries keyed by lemma.
pub fn parse_file(text: &str, out: &mut BTreeMap<String, Entry>) {
    let mut cur: Option<Raw> = None;
    for para in text.split("<p>") {
        let para = para.split("</p>").next().unwrap_or("");
        if let Some(c) = ENT.captures(para) {
            flush(cur.take(), out);
            let head = display(&clean(&c[1]));
            let mut raw = Raw { word: head, ..Default::default() };
            raw.pos = POS.captures(para).map(|p| clean(&p[1])).unwrap_or_default();
            raw.etymology = ETY.captures(para).map(|p| clean(&p[1])).unwrap_or_default();
            cur = Some(raw);
        }
        let Some(raw) = cur.as_mut() else { continue };
        // Definitions (a paragraph can hold several).
        for d in DEF.captures_iter(para) {
            let body = &d[1];
            let mut tags: Vec<String> = MARK.captures_iter(para).map(|m| clean(&m[1]).trim_matches(['[', ']']).to_string()).filter(|t| !t.is_empty()).collect();
            tags.dedup();
            let usage: Vec<Example> = AS.captures_iter(body).map(|a| Example { text: clean(&a[1]).trim_start_matches("as, ").to_string(), by: None }).filter(|e| !e.text.is_empty()).collect();
            // The gloss without the embedded "as, ..." usage.
            let gloss = clean(&AS.replace_all(body, ""));
            let gloss = gloss.trim_end_matches([';', ' ']).to_string();
            if gloss.is_empty() {
                continue;
            }
            raw.senses.push(Sense { gloss, tags: tags.into_iter().take(3).collect(), examples: usage, ..Default::default() });
        }
        // Quotations belong to the definition above them.
        if let (Some(last), true) = (raw.senses.last_mut(), para.contains("<q>")) {
            // The author is printed after the quotation, outside its tag.
            let by = QAU.captures(para).map(|a| clean(&a[1])).filter(|s| !s.is_empty());
            for q in QUOTE.captures_iter(para) {
                let text = clean(&q[1]);
                if !text.is_empty() && last.examples.len() < 3 {
                    last.examples.push(Example { text, by: by.clone() });
                }
            }
        }
    }
    flush(cur.take(), out);
}

pub fn build(dir: &Path, out_path: &Path) -> Result<()> {
    let mut files: Vec<_> = std::fs::read_dir(dir)?.flatten().map(|e| e.path()).filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("CIDE."))).collect();
    files.sort();
    if files.is_empty() {
        bail!("no CIDE.* files in {}", dir.display());
    }
    let mut entries = BTreeMap::new();
    for f in &files {
        parse_file(&String::from_utf8_lossy(&std::fs::read(f)?), &mut entries);
    }
    if out_path.exists() {
        std::fs::remove_file(out_path)?;
    }
    if let Some(d) = out_path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let conn = Connection::open(out_path)?;
    conn.pragma_update(None, "journal_mode", "OFF")?;
    conn.pragma_update(None, "synchronous", "OFF")?;
    conn.execute_batch(pack::SCHEMA)?;
    conn.execute_batch("BEGIN")?;
    let samples: Vec<Vec<u8>> = entries.values().step_by((entries.len() / 6000).max(1)).map(|e| serde_json::to_vec(e).unwrap()).collect();
    let dict = zstd::dict::from_samples(&samples, 64 * 1024)?;
    conn.execute("INSERT INTO meta VALUES('zdict', ?)", [&dict])?;
    let enc = zstd::dict::EncoderDictionary::copy(&dict, 12);
    for (k, e) in &entries {
        let blob = zstd::bulk::Compressor::with_prepared_dictionary(&enc)?.compress(&serde_json::to_vec(e)?)?;
        conn.execute("INSERT INTO entries(key,word,blob) VALUES (?,?,?)", params![k, e.word, blob])?;
    }
    conn.execute("INSERT INTO meta VALUES('source', 'Webster''s Revised Unabridged Dictionary (1913) via GCIDE 0.53, public domain')", [])?;
    conn.execute("INSERT INTO meta VALUES('entries', ?)", [entries.len().to_string()])?;
    conn.execute_batch("COMMIT; VACUUM;")?;
    eprintln!("{} entries from {} files", entries.len(), files.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
<p><ent>Loquacious</ent><br/
<hw>Lo*qua"cious</hw> <pr>(?)</pr>, <pos>a.</pos> <ety>[L. <ets>loquax</ets>, talkative, fr. <ets>loqui</ets> to speak.]</ety> <sn>1.</sn> <def>Given to continual talking; talkative; garrulous.</def><br/
[<source>1913 Webster</source>]</p>

<p><q><qex>Loquacious</qex>, brawling, ever in the wrong.</q> <rj><qau>Dryden.</qau></rj><br/
[<source>1913 Webster</source>]</p>

<p><sn>2.</sn> <def>Speaking; expressive.</def> <mark>[R.]</mark>  <rj><au>J. Philips.</au></rj><br/
[<source>1913 Webster</source>]</p>

<p><ent>Abate</ent><br/
<hw>A*bate"</hw> <pr>(<adot/*b<amac/t")</pr>, <pos>v. t.</pos> <sn>2.</sn> <def>To bring down; to lessen; <as>as, to <ex>abate</ex> a demand; to <ex>abate</ex> pride.</as></def><br/
[<source>1913 Webster</source>]</p>

<p><ent>Abate</ent><br/
<hw>A*bate"</hw>, <pos>v. i.</pos> <def>To decrease, or become less in strength; as, the storm <ex>abates</ex>.</def></p>

<p><ent>Caf<eacute/</ent><br/
<pos>n.</pos> <def>A coffee house.</def></p>
"#;

    #[test]
    fn cleans_markup_and_entities() {
        assert_eq!(clean(r#"(<adot/*b<amac/t")"#), r#"(*bt")"#);
        assert_eq!(clean("Caf<eacute/ <b>au</b> lait"), "Café au lait");
        assert_eq!(clean("<def>Given  to\n talking;</def>"), "Given to talking;");
    }

    #[test]
    fn parses_entries_senses_quotes_and_homographs() {
        let mut m = BTreeMap::new();
        parse_file(SAMPLE, &mut m);
        let l = &m["loquacious"];
        assert_eq!(l.blocks[0].pos, "adj");
        assert_eq!(l.blocks[0].senses.len(), 2);
        assert_eq!(l.blocks[0].senses[0].gloss, "Given to continual talking; talkative; garrulous.");
        // The quotation in the next paragraph belongs to the first sense.
        assert_eq!(l.blocks[0].senses[0].examples[0].by.as_deref(), Some("Dryden."));
        assert!(l.blocks[0].senses[0].examples[0].text.starts_with("Loquacious, brawling"));
        assert_eq!(l.blocks[0].senses[1].tags, vec!["R."]);
        assert!(l.blocks[0].etymology.as_ref().unwrap().contains("loquax"));
        // Two "Abate" entries (transitive, intransitive) become two blocks of one entry.
        let a = &m["abate"];
        assert_eq!(a.blocks.len(), 2);
        assert_eq!(a.blocks[0].senses[0].gloss, "To bring down; to lessen");
        assert_eq!(a.blocks[0].senses[0].examples[0].text, "to abate a demand; to abate pride.");
        assert_eq!(m["café"].blocks[0].pos, "noun");
        assert_eq!(l.word, "loquacious", "headwords are shown lowercase");
        assert_eq!(display("McCoy"), "McCoy");
    }
}
