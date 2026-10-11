//! Pronunciations from the CMU Pronouncing Dictionary (ARPAbet), converted to
//! IPA, as a fallback for words whose dictionary entry carries none.

use std::path::Path;

use anyhow::Result;
use rusqlite::{Connection, OpenFlags, OptionalExtension};

pub const SCHEMA: &str = "
CREATE TABLE meta(k TEXT PRIMARY KEY, v BLOB NOT NULL) WITHOUT ROWID;
CREATE TABLE pron(key TEXT PRIMARY KEY, ipa TEXT NOT NULL) WITHOUT ROWID;
";

fn is_vowel(p: &str) -> bool {
    p.trim_end_matches(|c: char| c.is_ascii_digit()).chars().next().is_some_and(|c| "AEIOU".contains(c))
}

fn symbol(p: &str) -> &'static str {
    let (base, stress) = p.split_at(p.trim_end_matches(|c: char| c.is_ascii_digit()).len());
    let unstressed = stress == "0";
    match base {
        "AA" => "ɑ",
        "AE" => "æ",
        "AH" => if unstressed { "ə" } else { "ʌ" },
        "AO" => "ɔ",
        "AW" => "aʊ",
        "AY" => "aɪ",
        "EH" => "ɛ",
        "ER" => if unstressed { "ɚ" } else { "ɝ" },
        "EY" => "eɪ",
        "IH" => "ɪ",
        "IY" => "i",
        "OW" => "oʊ",
        "OY" => "ɔɪ",
        "UH" => "ʊ",
        "UW" => "u",
        "B" => "b", "CH" => "tʃ", "D" => "d", "DH" => "ð", "F" => "f", "G" => "ɡ", "HH" => "h",
        "JH" => "dʒ", "K" => "k", "L" => "l", "M" => "m", "N" => "n", "NG" => "ŋ", "P" => "p",
        "R" => "ɹ", "S" => "s", "SH" => "ʃ", "T" => "t", "TH" => "θ", "V" => "v", "W" => "w",
        "Y" => "j", "Z" => "z", "ZH" => "ʒ",
        _ => "",
    }
}

/// "L OW0 K W EY1 SH AH0 S" -> "/loʊˈkweɪʃəs/". A stress mark goes before the
/// consonants that open the stressed syllable (up to two, as English onsets mostly are).
/// Consonant clusters English words can start with: stop/fricative + l r w y
/// (pl-, tr-, fy-), s + stop/nasal/l/w (sp-, sm-, sl-), and s + stop + l r w y (str-, spl-).
fn valid_onset(c: &[&str]) -> bool {
    const LIQ: &[&str] = &["L", "R", "W", "Y"];
    const FIRST: &[&str] = &["P", "B", "T", "D", "K", "G", "F", "V", "TH", "SH", "HH"];
    match c {
        [a, b] => (FIRST.contains(a) && LIQ.contains(b)) || (*a == "S" && ["P", "T", "K", "M", "N", "L", "W", "F"].contains(b)),
        [a, b, c] => *a == "S" && ["P", "T", "K"].contains(b) && LIQ.contains(c),
        _ => true,
    }
}

pub fn arpabet_to_ipa(arpa: &str) -> String {
    let ph: Vec<&str> = arpa.split_whitespace().collect();
    let mut out = String::new();
    for (i, p) in ph.iter().enumerate() {
        if is_vowel(p) && (p.ends_with('1') || p.ends_with('2')) {
            let mark = if p.ends_with('1') { 'ˈ' } else { 'ˌ' };
            // Walk back over the consonants that can open this syllable.
            let mut onset = 0;
            while onset < 3 && i > onset && !is_vowel(ph[i - 1 - onset]) {
                onset += 1;
            }
            while onset > 1 && !valid_onset(&ph[i - onset..i]) {
                onset -= 1;
            }
            // Insert before those consonants' IPA (they were just appended).
            let cut: usize = ph[i - onset..i].iter().map(|c| symbol(c).len()).sum();
            let at = out.len() - cut;
            out.insert(at, mark);
        }
        out.push_str(symbol(p));
    }
    format!("/{out}/")
}

pub struct Pronunciations {
    conn: Connection,
}

impl Pronunciations {
    pub fn open(path: &Path) -> Result<Self> {
        Ok(Pronunciations { conn: Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)? })
    }

    pub fn get(&self, key: &str) -> Result<Option<String>> {
        Ok(self.conn.query_row("SELECT ipa FROM pron WHERE key=?", [key], |r| r.get(0)).optional()?)
    }
}

#[cfg(test)]
mod tests {
    use super::arpabet_to_ipa;

    #[test]
    fn converts_with_stress() {
        assert_eq!(arpabet_to_ipa("L OW0 K W EY1 SH AH0 S"), "/loʊˈkweɪʃəs/");
        assert_eq!(arpabet_to_ipa("AH0 B EY1 T"), "/əˈbeɪt/");
        assert_eq!(arpabet_to_ipa("T AE1 S IH0 T ER2 N"), "/ˈtæsɪˌtɝn/");
        assert_eq!(arpabet_to_ipa("S T R EH1 NG TH"), "/ˈstɹɛŋθ/");
        assert_eq!(arpabet_to_ipa("AH0 K S EH1 P T"), "/əkˈsɛpt/");
    }
}
