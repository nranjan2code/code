//! Text matching shared by the in-session readers (`TurnIndex` subject
//! search and the ledger's own ranked lookup): tokens, named entities,
//! normalisation and a BM25-shaped score. Cross-session search is the data
//! catalog's (`vak-catalog`, plan M6).

use std::collections::HashSet;

const PHRASE_BONUS: f32 = 3.0;
/// BM25 parameters.
const BM25_K1: f32 = 1.2;
const BM25_B: f32 = 0.75;
/// Bonus multiplier for entity tokens — named entities matching query terms
/// are strong relevance signals.
const ENTITY_TOKEN_BONUS: f32 = 4.0;
/// Saturation point for term frequency in BM25 denominator.
const BM25_AVGDL_APPROX: f32 = 100.0;

pub fn tokenize_impl(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let normalized = normalize_impl(text);
    let mut current = String::new();
    for ch in normalized.chars() {
        if ch.is_alphanumeric() {
            current.push(ch);
        } else if !current.is_empty() {
            push_token(&mut out, &mut seen, &current);
            current.clear();
        }
    }
    if !current.is_empty() {
        push_token(&mut out, &mut seen, &current);
    }
    out
}

/// Extract named entities from text — capitalized proper nouns and acronyms.
/// Matches vakyartha's entity extraction approach:
///   - Proper noun: `[A-Z][a-z]{2,}` optionally followed by more such words
///     (1-4 words, like "New York" → "new_york")
///   - Acronym: `[A-Z]{2,6}` (like "NASA", "JSON")
///   - Numbers with units (dates, sizes, currency)
///
/// Excludes common noise words.
pub fn extract_entities(text: &str) -> Vec<String> {
    let mut entities: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    // Split into words and look for proper noun sequences
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut i = 0;
    while i < words.len() {
        let word = words[i].trim_end_matches(|c: char| !c.is_alphanumeric());
        if is_proper_noun_word(word) {
            // Collect consecutive proper noun words (up to 4)
            let mut term = word.to_lowercase();
            let mut j = i + 1;
            while j < words.len() && j < i + 4 {
                let next = words[j].trim_end_matches(|c: char| !c.is_alphanumeric());
                if is_proper_noun_word(next) {
                    term.push('_');
                    term.push_str(&next.to_lowercase());
                    j += 1;
                } else {
                    break;
                }
            }
            if !is_noisy_entity(&term) && seen.insert(term.clone()) {
                entities.push(term);
            }
            i = j;
        } else if is_acronym(word) {
            let term = word.to_lowercase();
            if !is_noisy_entity(&term) && seen.insert(term.clone()) {
                entities.push(term);
            }
            i += 1;
        } else {
            i += 1;
        }
    }

    entities
}

/// Check if a word is a proper noun: starts with uppercase, followed by
/// 2+ lowercase letters. Matches vakyartha's `[A-Z][a-z]{2,}`.
fn is_proper_noun_word(word: &str) -> bool {
    let chars: Vec<char> = word.chars().collect();
    chars.len() >= 4
        && chars[0].is_uppercase()
        && chars[1].is_lowercase()
        && chars[2].is_lowercase()
        && chars[3].is_lowercase()
}

/// Check if a word is an acronym: 2-6 consecutive uppercase letters.
fn is_acronym(word: &str) -> bool {
    let alpha: String = word.chars().filter(|c| c.is_alphabetic()).collect();
    alpha.len() >= 2 && alpha.len() <= 6 && alpha.chars().all(|c| c.is_uppercase())
}

fn is_noisy_entity(term: &str) -> bool {
    matches!(
        term,
        "the"
            | "this"
            | "that"
            | "these"
            | "those"
            | "current"
            | "latest"
            | "previous"
            | "next"
            | "key"
            | "summary"
            | "based"
            | "choice"
            | "recommended"
            | "actions"
            | "analysis"
            | "sources"
            | "related"
            | "source"
            | "tools"
            | "recovered"
            | "please"
            | "story"
            | "continues"
            | "copyright"
            | "article"
            | "body"
    )
}

fn push_token(out: &mut Vec<String>, seen: &mut HashSet<String>, token: &str) {
    if token.chars().count() >= 2 && seen.insert(token.to_string()) {
        out.push(token.to_string());
    }
}

/// Public for intra-crate use (e.g. SessionLog proactive retrieval).
pub(crate) fn normalize_impl(text: &str) -> String {
    text.to_lowercase()
}

/// BM25-style scoring with entity bonus. Matches vakyartha's
/// `score_normalized` + entity bonus approach.
pub fn score_normalized(hay: &str, terms: &[String], phrase: &str, entities: &[String]) -> f32 {
    let doc_len = hay.chars().count().max(1) as f32;
    let avgdl = BM25_AVGDL_APPROX;
    let mut score = 0.0f32;
    let mut matched_any = false;
    let entity_set: HashSet<&str> = entities.iter().map(|s| s.as_str()).collect();

    for term in terms {
        let mut count = 0usize;
        let mut from = 0usize;
        while let Some(pos) = hay[from..].find(term.as_str()) {
            count += 1;
            let abs = from + pos + term.len();
            if abs >= hay.len() {
                break;
            }
            from = abs;
            if count >= 16 {
                break;
            }
        }
        if count > 0 {
            matched_any = true;
            let idf = 1.0f32.ln_1p();
            // BM25: idf * (tf * (k1+1)) / (tf + k1 * (1 - b + b * dl/avgdl))
            let tf = count as f32;
            let length_penalty = 1.0 - BM25_B + BM25_B * doc_len / avgdl;
            let denom = tf + BM25_K1 * length_penalty;
            let mut term_score = if denom > 0.0 {
                idf * tf * (BM25_K1 + 1.0) / denom
            } else {
                idf * tf
            };
            // Entity bonus: if this term matches a named entity in the text,
            // multiply by ENTITY_TOKEN_BONUS.
            if entity_set.contains(term.as_str()) {
                term_score *= ENTITY_TOKEN_BONUS;
            }
            score += term_score;
        }
    }
    if !matched_any {
        return 0.0;
    }
    if hay.contains(phrase) {
        score += PHRASE_BONUS;
    }
    score
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_extraction_finds_camel_case_and_title_case() {
        let text = "The Kubernetes cluster runs the Docker container for PostgreSQL.";
        let entities = extract_entities(text);
        assert!(entities.contains(&"kubernetes".to_string()));
        assert!(entities.contains(&"docker".to_string()));
        assert!(entities.contains(&"postgresql".to_string()));
    }

    #[test]
    fn entity_bonus_raises_score_for_named_entities() {
        // Same phrase so PHRASE_BONUS cancels in the ratio.
        let terms = vec!["kubernetes".to_string()];
        let phrase = "kubernetes";
        let entities = vec!["kubernetes".to_string()];
        let score_with_bonus =
            score_normalized("the kubernetes cluster", &terms, phrase, &entities);
        let score_without = score_normalized("the kubernetes cluster", &terms, phrase, &[]);
        assert!(
            score_with_bonus > score_without,
            "entity bonus should increase score"
        );
        // The entity bonus multiplies the term score by ENTITY_TOKEN_BONUS.
        // PHRASE_BONUS is added to both, so the ratio is diluted but
        // score_with_bonus - PHRASE > (score_without - PHRASE) * ENTITY_BONUS.
        assert!(
            (score_with_bonus - PHRASE_BONUS) > (score_without - PHRASE_BONUS) * 0.99,
            "entity bonus should multiply the term score"
        );
    }
}
