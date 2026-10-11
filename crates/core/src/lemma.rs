//! Lemma keys: the stable identity of a word across dictionary packs.
//!
//! User data (history, notes, quiz items) references words only by this key,
//! never by pack row ids, so packs can be rebuilt or replaced on update.
//! Changing this function changes every stored key: treat it as a data
//! migration, not a refactor.

use unicode_normalization::UnicodeNormalization;

/// NFKC, lowercase, typographic apostrophes/dashes folded, whitespace collapsed.
/// Diacritics are kept ("blasé" and "blase" are distinct keys); fuzzy search
/// handles the folding at query time.
pub fn key(word: &str) -> String {
    let mut out = String::with_capacity(word.len());
    let mut pending_space = false;
    for c in word.nfkc().flat_map(char::to_lowercase) {
        let c = match c {
            '\u{2018}' | '\u{2019}' | '\u{02BC}' => '\'',
            '\u{2010}'..='\u{2015}' => '-',
            c => c,
        };
        if c.is_whitespace() {
            pending_space = !out.is_empty();
            continue;
        }
        if pending_space {
            out.push(' ');
            pending_space = false;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::key;

    #[test]
    fn normalizes() {
        assert_eq!(key("  Abate "), "abate");
        assert_eq!(key("de  facto"), "de facto");
        assert_eq!(key("o’clock"), "o'clock");
        assert_eq!(key("blasé"), "blasé");
        assert_eq!(key("self\u{2010}interest"), "self-interest");
    }
}
