//! The normalized entry every dictionary source is mapped into, so the UI is
//! independent of where an entry came from. Field names are short-lived JSON
//! keys inside zstd blobs; keep them stable (packs are rebuilt, not migrated,
//! but old packs must stay readable until the user updates them).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    pub word: String,
    /// The word broken into syllables with middle dots ("lo·qua·cious"), when the source says how.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub syl: Option<String>,
    /// One block per part of speech (and per etymology, for homographs).
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Block {
    /// Syllables as reported for this block; moved up to the entry when packs are built.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub syl: Option<String>,
    /// Set when this block belongs to a case variant of the entry's word
    /// ("Polish" under "polish").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub word: Option<String>,
    /// "noun", "verb", "adj", "adv", ... as the source spells it.
    pub pos: String,
    /// Inflection line, e.g. "dictionary (plural dictionaries)".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub etymology: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sounds: Vec<Sound>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub forms: Vec<Form>,
    pub senses: Vec<Sense>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub synonyms: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub antonyms: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Sound {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ipa: Option<String>,
    /// Remote audio URL (fetched and cached on first play when online).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<String>,
    /// Accent tags such as "US", "UK".
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Form {
    pub form: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Sense {
    pub gloss: String,
    /// Enclosing glosses for nested senses, outermost first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parents: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<Example>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub synonyms: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub antonyms: Vec<String>,
    /// Set for inflected forms: the lemma this sense points at.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub form_of: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Example {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    /// The example translated (English-to-Vietnamese pack).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tr: Option<String>,
}
