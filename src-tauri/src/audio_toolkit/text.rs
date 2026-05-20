use crate::settings::CorrectionPair;
use natural::phonetics::soundex;
use once_cell::sync::Lazy;
use regex::Regex;
use strsim::levenshtein;

/// Builds an n-gram string by cleaning and concatenating words
///
/// Strips punctuation from each word, lowercases, and joins without spaces.
/// This allows matching "Charge B" against "ChargeBee".
fn build_ngram(words: &[&str]) -> String {
    words
        .iter()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .collect::<Vec<_>>()
        .concat()
}

/// Finds the best matching custom word for a candidate string
///
/// Uses Levenshtein distance and Soundex phonetic matching to find
/// the best match above the given threshold.
///
/// # Arguments
/// * `candidate` - The cleaned/lowercased candidate string to match
/// * `custom_words` - Original custom words (for returning the replacement)
/// * `custom_words_nospace` - Custom words with spaces removed, lowercased (for comparison)
/// * `threshold` - Maximum similarity score to accept
///
/// # Returns
/// The best matching custom word and its score, if any match was found
fn find_best_match<'a>(
    candidate: &str,
    custom_words: &'a [String],
    custom_words_nospace: &[String],
    threshold: f64,
) -> Option<(&'a String, f64)> {
    if candidate.is_empty() || candidate.len() > 50 {
        return None;
    }

    let mut best_match: Option<&String> = None;
    let mut best_score = f64::MAX;

    for (i, custom_word_nospace) in custom_words_nospace.iter().enumerate() {
        // Skip if lengths are too different (optimization + prevents over-matching)
        // Use percentage-based check: max 25% length difference (prevents n-grams from
        // matching significantly shorter custom words, e.g., "openaigpt" vs "openai")
        let len_diff = (candidate.len() as i32 - custom_word_nospace.len() as i32).abs() as f64;
        let max_len = candidate.len().max(custom_word_nospace.len()) as f64;
        let max_allowed_diff = (max_len * 0.25).max(2.0); // At least 2 chars difference allowed
        if len_diff > max_allowed_diff {
            continue;
        }

        // Calculate Levenshtein distance (normalized by length)
        let levenshtein_dist = levenshtein(candidate, custom_word_nospace);
        let max_len = candidate.len().max(custom_word_nospace.len()) as f64;
        let levenshtein_score = if max_len > 0.0 {
            levenshtein_dist as f64 / max_len
        } else {
            1.0
        };

        // Calculate phonetic similarity using Soundex
        let phonetic_match = soundex(candidate, custom_word_nospace);

        // Combine scores: favor phonetic matches, but also consider string similarity
        let combined_score = if phonetic_match {
            levenshtein_score * 0.3 // Give significant boost to phonetic matches
        } else {
            levenshtein_score
        };

        // Accept if the score is good enough (configurable threshold)
        if combined_score < threshold && combined_score < best_score {
            best_match = Some(&custom_words[i]);
            best_score = combined_score;
        }
    }

    best_match.map(|m| (m, best_score))
}

/// Applies custom word corrections to transcribed text using fuzzy matching
///
/// This function corrects words in the input text by finding the best matches
/// from a list of custom words using a combination of:
/// - Levenshtein distance for string similarity
/// - Soundex phonetic matching for pronunciation similarity
/// - N-gram matching for multi-word speech artifacts (e.g., "Charge B" -> "ChargeBee")
///
/// # Arguments
/// * `text` - The input text to correct
/// * `custom_words` - List of custom words to match against
/// * `threshold` - Maximum similarity score to accept (0.0 = exact match, 1.0 = any match)
///
/// # Returns
/// The corrected text with custom words applied
pub fn apply_custom_words(text: &str, custom_words: &[String], threshold: f64) -> String {
    if custom_words.is_empty() {
        return text.to_string();
    }

    // Pre-compute lowercase versions to avoid repeated allocations
    let custom_words_lower: Vec<String> = custom_words.iter().map(|w| w.to_lowercase()).collect();

    // Pre-compute versions with spaces removed for n-gram comparison
    let custom_words_nospace: Vec<String> = custom_words_lower
        .iter()
        .map(|w| w.replace(' ', ""))
        .collect();

    let words: Vec<&str> = text.split_whitespace().collect();
    let mut result = Vec::new();
    let mut i = 0;

    while i < words.len() {
        let mut matched = false;

        // Try n-grams from longest (3) to shortest (1) - greedy matching
        for n in (1..=3).rev() {
            if i + n > words.len() {
                continue;
            }

            let ngram_words = &words[i..i + n];
            let ngram = build_ngram(ngram_words);

            if let Some((replacement, _score)) =
                find_best_match(&ngram, custom_words, &custom_words_nospace, threshold)
            {
                // Extract punctuation from first and last words of the n-gram
                let (prefix, _) = extract_punctuation(ngram_words[0]);
                let (_, suffix) = extract_punctuation(ngram_words[n - 1]);

                // Preserve case from first word
                let corrected = preserve_case_pattern(ngram_words[0], replacement);

                result.push(format!("{}{}{}", prefix, corrected, suffix));
                i += n;
                matched = true;
                break;
            }
        }

        if !matched {
            result.push(words[i].to_string());
            i += 1;
        }
    }

    result.join(" ")
}

/// Preserves the case pattern of the original word when applying a replacement
fn preserve_case_pattern(original: &str, replacement: &str) -> String {
    if original.chars().all(|c| c.is_uppercase()) {
        replacement.to_uppercase()
    } else if original.chars().next().map_or(false, |c| c.is_uppercase()) {
        let mut chars: Vec<char> = replacement.chars().collect();
        if let Some(first_char) = chars.get_mut(0) {
            *first_char = first_char.to_uppercase().next().unwrap_or(*first_char);
        }
        chars.into_iter().collect()
    } else {
        replacement.to_string()
    }
}

/// Extracts punctuation prefix and suffix from a word
fn extract_punctuation(word: &str) -> (&str, &str) {
    let prefix_end = word.chars().take_while(|c| !c.is_alphanumeric()).count();
    let suffix_start = word
        .char_indices()
        .rev()
        .take_while(|(_, c)| !c.is_alphanumeric())
        .count();

    let prefix = if prefix_end > 0 {
        &word[..prefix_end]
    } else {
        ""
    };

    let suffix = if suffix_start > 0 {
        &word[word.len() - suffix_start..]
    } else {
        ""
    };

    (prefix, suffix)
}

/// Returns filler words appropriate for the given language code.
///
/// Some words like "um" and "ha" are real words in certain languages
/// (e.g., Portuguese "um" = "a/an", Spanish "ha" = "has"), so we only
/// include them as fillers for languages where they are truly fillers.
fn get_filler_words_for_language(lang: &str) -> &'static [&'static str] {
    let base_lang = lang.split(&['-', '_'][..]).next().unwrap_or(lang);

    match base_lang {
        "en" => &[
            "uh", "um", "uhm", "umm", "uhh", "uhhh", "ah", "hmm", "hm", "mmm", "mm", "mh", "eh",
            "ehh", "ha",
        ],
        "es" => &["ehm", "mmm", "hmm", "hm"],
        "pt" => &["ahm", "hmm", "mmm", "hm"],
        "fr" => &["euh", "hmm", "hm", "mmm"],
        "de" => &["äh", "ähm", "hmm", "hm", "mmm"],
        "it" => &["ehm", "hmm", "mmm", "hm"],
        "cs" => &["ehm", "hmm", "mmm", "hm"],
        "pl" => &["hmm", "mmm", "hm"],
        "tr" => &["hmm", "mmm", "hm"],
        "ru" => &["хм", "ммм", "hmm", "mmm"],
        "uk" => &["хм", "ммм", "hmm", "mmm"],
        "ar" => &["hmm", "mmm"],
        "ja" => &["hmm", "mmm"],
        "ko" => &["hmm", "mmm"],
        "vi" => &["hmm", "mmm", "hm"],
        "zh" => &["hmm", "mmm"],
        // Conservative universal fallback (no "um", "eh", "ha")
        _ => &[
            "uh", "uhm", "umm", "uhh", "uhhh", "ah", "hmm", "hm", "mmm", "mm", "mh", "ehh",
        ],
    }
}

static MULTI_SPACE_PATTERN: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s{2,}").unwrap());

/// Returns true if the word is a short consonant-only cluster (no vowels, ≤3 chars)
/// that looks like a speech stutter fragment (e.g., "s", "wh", "th").
/// Words with vowels — including common short words like "I", "a", "in", "so" — return false.
fn is_consonant_fragment(word: &str) -> bool {
    !word.is_empty()
        && word.len() <= 3
        && word.chars().all(|c| c.is_alphabetic())
        && !word.chars().any(|c| matches!(c, 'a' | 'e' | 'i' | 'o' | 'u'))
}

/// Collapses repeated words (3+ repetitions) to a single instance, and removes
/// consonant-only stutter fragments that precede the word being attempted.
/// E.g., "wh wh wh wh" -> "wh", "I I I I" -> "I",
///       "s so" -> "so", "wh wh when" -> "when"
fn collapse_stutters(text: &str) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return text.to_string();
    }

    let mut result: Vec<&str> = Vec::new();
    let mut i = 0;

    while i < words.len() {
        let word = words[i];
        let word_lower = word.to_lowercase();

        if !word_lower.chars().all(|c| c.is_alphabetic()) {
            result.push(word);
            i += 1;
            continue;
        }

        // Count consecutive identical repetitions (case-insensitive)
        let mut count = 1;
        while i + count < words.len() && words[i + count].to_lowercase() == word_lower {
            count += 1;
        }

        let after = i + count; // index of first word after all identical repetitions

        // 3+ identical repetitions: collapse to one instance.
        // If the following word starts with this consonant fragment (e.g., "wh wh wh when"),
        // emit the full word instead.
        if count >= 3 {
            if is_consonant_fragment(&word_lower) && after < words.len() {
                let next_lower = words[after].to_lowercase();
                if next_lower.starts_with(&word_lower) && next_lower.len() > word_lower.len() {
                    result.push(words[after]);
                    i = after + 1;
                    continue;
                }
            }
            result.push(word);
            i += count;
            continue;
        }

        // For consonant-only fragments with count < 3, check if the word following
        // all repetitions is the full word being stuttered toward.
        // e.g., "s so" → "so", "wh wh when" → "when"
        if is_consonant_fragment(&word_lower) && after < words.len() {
            let next_lower = words[after].to_lowercase();
            if next_lower.starts_with(&word_lower) && next_lower.len() > word_lower.len() {
                result.push(words[after]);
                i = after + 1;
                continue;
            }
        }

        result.push(word);
        i += 1;
    }

    result.join(" ")
}

/// Filters transcription output by removing filler words and stutter artifacts.
///
/// This function cleans up raw transcription text by:
/// 1. Removing filler words based on the app language (or custom list)
/// 2. Collapsing repeated word stutters (e.g., "wh wh wh" -> "wh")
/// 3. Cleaning up excess whitespace
///
/// # Arguments
/// * `text` - The raw transcription text to filter
/// * `lang` - The app language code (e.g., "en", "pt-BR") used to select filler words
/// * `custom_filler_words` - Optional user-provided filler word list. `Some(vec)` overrides
///   language defaults; `Some(empty vec)` disables filtering; `None` uses language defaults.
///
/// # Returns
/// The filtered text with filler words and stutters removed
pub fn filter_transcription_output(
    text: &str,
    lang: &str,
    custom_filler_words: &Option<Vec<String>>,
) -> String {
    let mut filtered = text.to_string();

    // Build filler patterns from custom list or language defaults
    let patterns: Vec<Regex> = match custom_filler_words {
        Some(words) => words
            .iter()
            .filter_map(|word| Regex::new(&format!(r"(?i)\b{}\b[,.]?", regex::escape(word))).ok())
            .collect(),
        None => get_filler_words_for_language(lang)
            .iter()
            .map(|word| Regex::new(&format!(r"(?i)\b{}\b[,.]?", regex::escape(word))).unwrap())
            .collect(),
    };

    // Remove filler words
    for pattern in &patterns {
        filtered = pattern.replace_all(&filtered, "").to_string();
    }

    // Collapse repeated 1-2 letter words (stutter artifacts like "wh wh wh wh")
    filtered = collapse_stutters(&filtered);

    // Clean up multiple spaces to single space
    filtered = MULTI_SPACE_PATTERN.replace_all(&filtered, " ").to_string();

    // Trim leading/trailing whitespace
    filtered.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_apply_custom_words_exact_match() {
        let text = "hello world";
        let custom_words = vec!["Hello".to_string(), "World".to_string()];
        let result = apply_custom_words(text, &custom_words, 0.5);
        assert_eq!(result, "Hello World");
    }

    #[test]
    fn test_apply_custom_words_fuzzy_match() {
        let text = "helo wrold";
        let custom_words = vec!["hello".to_string(), "world".to_string()];
        let result = apply_custom_words(text, &custom_words, 0.5);
        assert_eq!(result, "hello world");
    }

    #[test]
    fn test_preserve_case_pattern() {
        assert_eq!(preserve_case_pattern("HELLO", "world"), "WORLD");
        assert_eq!(preserve_case_pattern("Hello", "world"), "World");
        assert_eq!(preserve_case_pattern("hello", "WORLD"), "WORLD");
    }

    #[test]
    fn test_extract_punctuation() {
        assert_eq!(extract_punctuation("hello"), ("", ""));
        assert_eq!(extract_punctuation("!hello?"), ("!", "?"));
        assert_eq!(extract_punctuation("...hello..."), ("...", "..."));
    }

    #[test]
    fn test_empty_custom_words() {
        let text = "hello world";
        let custom_words = vec![];
        let result = apply_custom_words(text, &custom_words, 0.5);
        assert_eq!(result, "hello world");
    }

    #[test]
    fn test_filter_filler_words() {
        let text = "So uhm I was thinking uh about this";
        let result = filter_transcription_output(text, "en", &None);
        assert_eq!(result, "So I was thinking about this");
    }

    #[test]
    fn test_filter_filler_words_case_insensitive() {
        let text = "UHM this is UH a test";
        let result = filter_transcription_output(text, "en", &None);
        assert_eq!(result, "this is a test");
    }

    #[test]
    fn test_filter_filler_words_with_punctuation() {
        let text = "Well, uhm, I think, uh. that's right";
        let result = filter_transcription_output(text, "en", &None);
        assert_eq!(result, "Well, I think, that's right");
    }

    #[test]
    fn test_filter_cleans_whitespace() {
        let text = "Hello    world   test";
        let result = filter_transcription_output(text, "en", &None);
        assert_eq!(result, "Hello world test");
    }

    #[test]
    fn test_filter_trims() {
        let text = "  Hello world  ";
        let result = filter_transcription_output(text, "en", &None);
        assert_eq!(result, "Hello world");
    }

    #[test]
    fn test_filter_combined() {
        let text = "  Uhm, so I was, uh, thinking about this  ";
        let result = filter_transcription_output(text, "en", &None);
        assert_eq!(result, "so I was, thinking about this");
    }

    #[test]
    fn test_filter_preserves_valid_text() {
        let text = "This is a completely normal sentence.";
        let result = filter_transcription_output(text, "en", &None);
        assert_eq!(result, "This is a completely normal sentence.");
    }

    #[test]
    fn test_filter_stutter_collapse() {
        // "w" is a fragment for "wh"; the 8× "wh"s are fragments for "why"
        let text = "w wh wh wh wh wh wh wh wh wh why";
        let result = filter_transcription_output(text, "en", &None);
        assert_eq!(result, "wh why");
    }

    #[test]
    fn test_filter_stutter_partial_then_full() {
        // single consonant fragment before full word
        let text = "s so I was thinking";
        let result = filter_transcription_output(text, "en", &None);
        assert_eq!(result, "so I was thinking");
    }

    #[test]
    fn test_filter_stutter_two_fragments_then_full() {
        // two identical fragments before full word
        let text = "wh wh when";
        let result = filter_transcription_output(text, "en", &None);
        assert_eq!(result, "when");
    }

    #[test]
    fn test_filter_stutter_fragment_preserves_real_short_words() {
        // "so", "I", "a" have vowels and must not be treated as fragments
        let text = "so I saw a dog";
        let result = filter_transcription_output(text, "en", &None);
        assert_eq!(result, "so I saw a dog");
    }

    #[test]
    fn test_filter_stutter_short_words() {
        let text = "I I I I think so so so so";
        let result = filter_transcription_output(text, "en", &None);
        assert_eq!(result, "I think so");
    }

    #[test]
    fn test_filter_stutter_longer_words() {
        let text = "Check data doc doc doc doc documentation.";
        let result = filter_transcription_output(text, "en", &None);
        assert_eq!(result, "Check data doc documentation.");
    }

    #[test]
    fn test_filter_stutter_mixed_case() {
        let text = "No NO no NO no";
        let result = filter_transcription_output(text, "en", &None);
        assert_eq!(result, "No");
    }

    #[test]
    fn test_filter_stutter_preserves_two_repetitions() {
        let text = "no no is fine";
        let result = filter_transcription_output(text, "en", &None);
        assert_eq!(result, "no no is fine");
    }

    #[test]
    fn test_filter_english_removes_um() {
        let text = "um I think um this is good";
        let result = filter_transcription_output(text, "en", &None);
        assert_eq!(result, "I think this is good");
    }

    #[test]
    fn test_filter_portuguese_preserves_um() {
        // "um" means "a/an" in Portuguese
        let text = "um gato bonito";
        let result = filter_transcription_output(text, "pt", &None);
        assert_eq!(result, "um gato bonito");
    }

    #[test]
    fn test_filter_spanish_preserves_ha() {
        // "ha" means "has" in Spanish
        let text = "ha sido un buen día";
        let result = filter_transcription_output(text, "es", &None);
        assert_eq!(result, "ha sido un buen día");
    }

    #[test]
    fn test_filter_language_code_with_region() {
        // "pt-BR" should normalize to "pt"
        let text = "um gato bonito";
        let result = filter_transcription_output(text, "pt-BR", &None);
        assert_eq!(result, "um gato bonito");
    }

    #[test]
    fn test_filter_custom_filler_words_override() {
        let custom = Some(vec!["okay".to_string(), "right".to_string()]);
        let text = "okay so I think right this works";
        let result = filter_transcription_output(text, "en", &custom);
        assert_eq!(result, "so I think this works");
    }

    #[test]
    fn test_filter_custom_filler_words_empty_disables() {
        let custom = Some(vec![]);
        let text = "So uhm I was thinking uh about this";
        let result = filter_transcription_output(text, "en", &custom);
        // No filler words removed since custom list is empty
        assert_eq!(result, "So uhm I was thinking uh about this");
    }

    #[test]
    fn test_filter_unknown_language_uses_fallback() {
        let text = "uh I think uhm this works";
        let result = filter_transcription_output(text, "xx", &None);
        assert_eq!(result, "I think this works");
    }

    #[test]
    fn test_filter_fallback_does_not_remove_um() {
        // Fallback (unknown language) should not remove "um" since it's a real word in some languages
        let text = "um I think this works";
        let result = filter_transcription_output(text, "xx", &None);
        assert_eq!(result, "um I think this works");
    }

    #[test]
    fn test_apply_custom_words_ngram_two_words() {
        let text = "il cui nome è Charge B, che permette";
        let custom_words = vec!["ChargeBee".to_string()];
        let result = apply_custom_words(text, &custom_words, 0.5);
        assert!(result.contains("ChargeBee,"));
        assert!(!result.contains("Charge B"));
    }

    #[test]
    fn test_apply_custom_words_ngram_three_words() {
        let text = "use Chat G P T for this";
        let custom_words = vec!["ChatGPT".to_string()];
        let result = apply_custom_words(text, &custom_words, 0.5);
        assert!(result.contains("ChatGPT"));
    }

    #[test]
    fn test_apply_custom_words_prefers_longer_ngram() {
        let text = "Open AI GPT model";
        let custom_words = vec!["OpenAI".to_string(), "GPT".to_string()];
        let result = apply_custom_words(text, &custom_words, 0.5);
        assert_eq!(result, "OpenAI GPT model");
    }

    #[test]
    fn test_apply_custom_words_ngram_preserves_case() {
        let text = "CHARGE B is great";
        let custom_words = vec!["ChargeBee".to_string()];
        let result = apply_custom_words(text, &custom_words, 0.5);
        assert!(result.contains("CHARGEBEE"));
    }

    #[test]
    fn test_apply_custom_words_ngram_with_spaces_in_custom() {
        // Custom word with space should also match against split words
        let text = "using Mac Book Pro";
        let custom_words = vec!["MacBook Pro".to_string()];
        let result = apply_custom_words(text, &custom_words, 0.5);
        assert!(result.contains("MacBook"));
    }

    // -------------------------------------------------------------------------
    // convert_number_words
    // -------------------------------------------------------------------------

    #[test]
    fn test_number_basic_cardinals() {
        assert_eq!(convert_number_words("twenty three items"), "23 items");
        assert_eq!(convert_number_words("five"), "5");
        assert_eq!(convert_number_words("zero"), "0");
        assert_eq!(convert_number_words("nineteen"), "19");
        assert_eq!(convert_number_words("ninety"), "90");
    }

    #[test]
    fn test_number_hundreds() {
        assert_eq!(convert_number_words("one hundred"), "100");
        assert_eq!(convert_number_words("three hundred"), "300");
        assert_eq!(convert_number_words("one hundred and twenty three"), "123");
        assert_eq!(convert_number_words("two hundred and fifty"), "250");
    }

    #[test]
    fn test_number_thousands_and_large() {
        assert_eq!(convert_number_words("two thousand"), "2000");
        assert_eq!(convert_number_words("twenty three thousand four hundred and fifty six"), "23456");
        assert_eq!(convert_number_words("one million"), "1000000");
        assert_eq!(convert_number_words("two hundred thousand"), "200000");
    }

    #[test]
    fn test_number_decimals() {
        assert_eq!(convert_number_words("three point five"), "3.5");
        assert_eq!(convert_number_words("thirty two point seven five"), "32.75");
        assert_eq!(convert_number_words("one point two three"), "1.23");
        // "point" alone (no following digit) stays unconsumed
        assert_eq!(convert_number_words("make a point about this"), "make a point about this");
    }

    #[test]
    fn test_number_ordinals_in_sequence() {
        assert_eq!(convert_number_words("twenty first floor"), "21st floor");
        assert_eq!(convert_number_words("twenty second"), "22nd");
        assert_eq!(convert_number_words("thirty third"), "33rd");
        assert_eq!(convert_number_words("one hundredth"), "100th");
    }

    #[test]
    fn test_number_standalone_ordinals_unchanged() {
        // Standalone ordinals must not be converted — too ambiguous
        assert_eq!(convert_number_words("give me a second"), "give me a second");
        assert_eq!(convert_number_words("first and foremost"), "first and foremost");
        assert_eq!(convert_number_words("the second opinion"), "the second opinion");
    }

    #[test]
    fn test_number_negatives() {
        assert_eq!(convert_number_words("negative twenty"), "-20");
        assert_eq!(convert_number_words("minus five"), "-5");
        assert_eq!(convert_number_words("minus one hundred"), "-100");
        // "negative" with no following number → unchanged
        assert_eq!(convert_number_words("negative"), "negative");
    }

    #[test]
    fn test_number_a_before_scale() {
        assert_eq!(convert_number_words("a hundred"), "100");
        assert_eq!(convert_number_words("a thousand users"), "1000 users");
        // "a" followed by a non-scale word → unchanged
        assert_eq!(convert_number_words("a second"), "a second");
    }

    #[test]
    fn test_number_punctuation_preserved() {
        assert_eq!(convert_number_words("twenty,"), "20,");
        assert_eq!(convert_number_words("five."), "5.");
        assert_eq!(convert_number_words("(twenty three)"), "(23)");
    }

    #[test]
    fn test_number_mixed_text() {
        assert_eq!(
            convert_number_words("I need twenty three items"),
            "I need 23 items"
        );
        assert_eq!(
            convert_number_words("page one hundred and fifty"),
            "page 150"
        );
        assert_eq!(
            convert_number_words("temperature is minus five degrees"),
            "temperature is -5 degrees"
        );
    }

    #[test]
    fn test_number_trailing_and_not_consumed() {
        // "one hundred and" — the trailing "and" should be left as-is
        assert_eq!(convert_number_words("one hundred and then"), "100 and then");
    }

    #[test]
    fn test_number_already_digits_unchanged() {
        assert_eq!(convert_number_words("I have 3 items"), "I have 3 items");
        assert_eq!(convert_number_words("version 1.5"), "version 1.5");
    }

    #[test]
    fn test_number_bare_scales_unchanged() {
        // "hundred" / "million" without a preceding number word — leave unchanged
        assert_eq!(convert_number_words("hundred"), "hundred");
        assert_eq!(convert_number_words("million dollar idea"), "million dollar idea");
    }

    #[test]
    fn test_apply_custom_words_trailing_number_not_doubled() {
        // Verify that trailing non-alpha chars (like numbers) aren't double-counted
        // between build_ngram stripping them and extract_punctuation capturing them
        let text = "use GPT4 for this";
        let custom_words = vec!["GPT-4".to_string()];
        let result = apply_custom_words(text, &custom_words, 0.5);
        // Should NOT produce "GPT-44" (double-counting the trailing 4)
        assert!(
            !result.contains("GPT-44"),
            "got double-counted result: {}",
            result
        );
    }
}

// ==============================================================================
// Number word → digit conversion
// ==============================================================================

/// Returns the ordinal suffix for a number (e.g. 1 → "st", 2 → "nd", 3 → "rd").
fn ordinal_suffix_for(n: u64) -> &'static str {
    match n % 100 {
        11 | 12 | 13 => "th",
        _ => match n % 10 {
            1 => "st",
            2 => "nd",
            3 => "rd",
            _ => "th",
        },
    }
}

/// Strips leading and trailing non-alphanumeric characters from a word.
fn word_core(word: &str) -> &str {
    word.trim_matches(|c: char| !c.is_alphanumeric())
}

/// The numeric role a single (cleaned) word can play in a number phrase.
#[derive(Clone, Copy)]
enum NumWord {
    /// 0–19  (zero, one, …, nineteen)
    Ones(u64),
    /// 20–90 in multiples of ten (twenty, thirty, …, ninety)
    Tens(u64),
    /// The word "hundred"
    Hundred,
    /// A large scale multiplier: thousand / million / billion
    BigScale(u64),
    /// "and" — ignored connector between digit groups
    Connector,
    /// "point" — decimal separator
    Point,
    /// An ordinal word (first=1, second=2, …, ninetieth=90).
    /// Standalone ordinals are left unchanged; they are only converted when
    /// they appear at the end of a multi-word number sequence.
    Ordinal(u64),
    /// A scale word that also implies an ordinal suffix (hundredth / thousandth).
    /// Like Hundred / BigScale it multiplies or accumulates the running total,
    /// but the result gets an ordinal suffix.  "one hundredth" → "100th".
    ScaleOrdinal(u64),
}

/// Maps a lowercase word to its [`NumWord`] role, or `None` if it is not a
/// recognised number word.
fn classify_number_word(word: &str) -> Option<NumWord> {
    Some(match word {
        "zero"        => NumWord::Ones(0),
        "one"         => NumWord::Ones(1),
        "two"         => NumWord::Ones(2),
        "three"       => NumWord::Ones(3),
        "four"        => NumWord::Ones(4),
        "five"        => NumWord::Ones(5),
        "six"         => NumWord::Ones(6),
        "seven"       => NumWord::Ones(7),
        "eight"       => NumWord::Ones(8),
        "nine"        => NumWord::Ones(9),
        "ten"         => NumWord::Ones(10),
        "eleven"      => NumWord::Ones(11),
        "twelve"      => NumWord::Ones(12),
        "thirteen"    => NumWord::Ones(13),
        "fourteen"    => NumWord::Ones(14),
        "fifteen"     => NumWord::Ones(15),
        "sixteen"     => NumWord::Ones(16),
        "seventeen"   => NumWord::Ones(17),
        "eighteen"    => NumWord::Ones(18),
        "nineteen"    => NumWord::Ones(19),
        "twenty"      => NumWord::Tens(20),
        "thirty"      => NumWord::Tens(30),
        "forty"       => NumWord::Tens(40),
        "fifty"       => NumWord::Tens(50),
        "sixty"       => NumWord::Tens(60),
        "seventy"     => NumWord::Tens(70),
        "eighty"      => NumWord::Tens(80),
        "ninety"      => NumWord::Tens(90),
        "hundred"     => NumWord::Hundred,
        "thousand"    => NumWord::BigScale(1_000),
        "million"     => NumWord::BigScale(1_000_000),
        "billion"     => NumWord::BigScale(1_000_000_000),
        "and"         => NumWord::Connector,
        "point"       => NumWord::Point,
        // Ordinals — only valid at the tail of a multi-word number sequence
        "first"       => NumWord::Ordinal(1),
        "second"      => NumWord::Ordinal(2),
        "third"       => NumWord::Ordinal(3),
        "fourth"      => NumWord::Ordinal(4),
        "fifth"       => NumWord::Ordinal(5),
        "sixth"       => NumWord::Ordinal(6),
        "seventh"     => NumWord::Ordinal(7),
        "eighth"      => NumWord::Ordinal(8),
        "ninth"       => NumWord::Ordinal(9),
        "tenth"       => NumWord::Ordinal(10),
        "eleventh"    => NumWord::Ordinal(11),
        "twelfth"     => NumWord::Ordinal(12),
        "thirteenth"  => NumWord::Ordinal(13),
        "fourteenth"  => NumWord::Ordinal(14),
        "fifteenth"   => NumWord::Ordinal(15),
        "sixteenth"   => NumWord::Ordinal(16),
        "seventeenth" => NumWord::Ordinal(17),
        "eighteenth"  => NumWord::Ordinal(18),
        "nineteenth"  => NumWord::Ordinal(19),
        "twentieth"   => NumWord::Ordinal(20),
        "thirtieth"   => NumWord::Ordinal(30),
        "fortieth"    => NumWord::Ordinal(40),
        "fiftieth"    => NumWord::Ordinal(50),
        "sixtieth"    => NumWord::Ordinal(60),
        "seventieth"  => NumWord::Ordinal(70),
        "eightieth"   => NumWord::Ordinal(80),
        "ninetieth"   => NumWord::Ordinal(90),
        "hundredth"   => NumWord::ScaleOrdinal(100),
        "thousandth"  => NumWord::ScaleOrdinal(1_000),
        _ => return None,
    })
}

/// Parses a run of number words beginning at `start`, returning
/// `(value, is_ordinal, words_consumed)` or `None` if no number words are
/// found.
///
/// Rules:
/// - "and" is accepted as a connector *within* an established number run, but
///   is rolled back if nothing follows it.
/// - Ordinals (first, second …) are accepted only at the *end* of a run that
///   already has at least one cardinal word — standalone ordinals are left
///   unchanged because they are too context-dependent.
/// - ScaleOrdinals (hundredth / thousandth) multiply the running value exactly
///   like their cardinal cousins but also set the ordinal flag.
/// - "a" is treated as 1 when immediately followed by "hundred", "thousand",
///   "million", or "billion" (e.g. "a hundred" → 100).
fn parse_integer_body(words: &[&str], start: usize) -> Option<(u64, bool, usize)> {
    let mut total: u64 = 0;
    let mut current: u64 = 0;
    let mut count: usize = 0;
    let mut is_ordinal = false;
    let mut pending_connector = false;

    // "a hundred / a thousand / …" — treat "a" as 1
    if start < words.len() {
        let wc = word_core(words[start]).to_lowercase();
        if wc == "a" && start + 1 < words.len() {
            let next_wc = word_core(words[start + 1]).to_lowercase();
            if matches!(
                classify_number_word(&next_wc),
                Some(NumWord::Hundred) | Some(NumWord::BigScale(_))
            ) {
                current = 1;
                count = 1;
            }
        }
    }

    loop {
        let idx = start + count;
        if idx >= words.len() {
            break;
        }
        let wc = word_core(words[idx]).to_lowercase();

        match classify_number_word(&wc) {
            None => break,
            Some(NumWord::Point) => break, // handled by caller
            Some(NumWord::Connector) => {
                if count == 0 {
                    break; // "and" with nothing before it — not a number
                }
                pending_connector = true;
                count += 1;
            }
            Some(NumWord::Ordinal(v)) => {
                // Only accept ordinals at the end of an established run
                if count == 0 {
                    break;
                }
                current += v;
                is_ordinal = true;
                pending_connector = false;
                count += 1;
                break; // ordinal always ends the sequence
            }
            Some(NumWord::Ones(v) | NumWord::Tens(v)) => {
                current += v;
                pending_connector = false;
                count += 1;
            }
            Some(NumWord::Hundred) => {
                if count == 0 {
                    break; // bare "hundred" — not a number
                }
                if current == 0 {
                    current = 1;
                }
                current *= 100;
                pending_connector = false;
                count += 1;
            }
            Some(NumWord::BigScale(scale)) => {
                if count == 0 {
                    break; // bare "million" etc. — not a number
                }
                let mult = if current == 0 { 1 } else { current };
                total += mult * scale;
                current = 0;
                pending_connector = false;
                count += 1;
            }
            Some(NumWord::ScaleOrdinal(scale)) => {
                // Like Hundred / BigScale but makes the result ordinal.
                // "one hundredth" → current*=100 → 100th
                // "two thousandth" → total += 2*1000 → 2000th
                if count == 0 {
                    break;
                }
                if scale == 100 {
                    if current == 0 {
                        current = 1;
                    }
                    current *= 100;
                } else {
                    let mult = if current == 0 { 1 } else { current };
                    total += mult * scale;
                    current = 0;
                }
                is_ordinal = true;
                pending_connector = false;
                count += 1;
                break; // scale-ordinal always ends the sequence
            }
        }
    }

    // Roll back a trailing "and" that had nothing following it
    if pending_connector && count > 0 {
        count -= 1;
    }

    if count == 0 {
        return None;
    }

    Some((total + current, is_ordinal, count))
}

/// Attempts to parse a number phrase starting at position `start` in `words`.
///
/// Returns `(converted_string, words_consumed)` or `None`.
fn try_parse_number(words: &[&str], start: usize) -> Option<(String, usize)> {
    let mut pos = start;

    // Preserve any leading punctuation attached to the first word (e.g. "($twenty")
    let (lead_punct, _) = extract_punctuation(words[pos]);

    // Optional negative / minus prefix
    let first_core = word_core(words[pos]).to_lowercase();
    let negative = matches!(first_core.as_str(), "negative" | "minus");
    if negative {
        // Only treat as a prefix when a number word immediately follows
        let next = pos + 1;
        if next >= words.len() {
            return None;
        }
        let next_lower = word_core(words[next]).to_lowercase();
        if classify_number_word(&next_lower).is_none() {
            return None;
        }
        pos += 1;
    }

    // Parse the integer body
    let (int_val, is_ordinal, body_count) = parse_integer_body(words, pos)?;
    pos += body_count;

    // Optional decimal: "point" followed by single-digit number words
    let mut decimal = String::new();
    if !is_ordinal && pos < words.len() {
        let pw = word_core(words[pos]).to_lowercase();
        if pw == "point" {
            let mut frac = String::new();
            let mut fp = pos + 1;
            while fp < words.len() {
                let dw = word_core(words[fp]).to_lowercase();
                match classify_number_word(&dw) {
                    Some(NumWord::Ones(d)) if d <= 9 => {
                        frac.push_str(&d.to_string());
                        fp += 1;
                    }
                    _ => break,
                }
            }
            if !frac.is_empty() {
                decimal = frac;
                pos = fp; // advance past "point" + digit words
            }
            // If no digit words follow "point", leave it unconsumed
        }
    }

    let total_consumed = pos - start;
    if total_consumed == 0 {
        return None;
    }

    // Trailing punctuation from the last consumed word
    let (_, trail_punct) = extract_punctuation(words[start + total_consumed - 1]);

    let sign = if negative { "-" } else { "" };
    let num = if !decimal.is_empty() {
        format!("{}{}.{}", sign, int_val, decimal)
    } else if is_ordinal {
        let suf = ordinal_suffix_for(int_val);
        format!("{}{}{}", sign, int_val, suf)
    } else {
        format!("{}{}", sign, int_val)
    };

    Some((format!("{}{}{}", lead_punct, num, trail_punct), total_consumed))
}

/// Post-processing pass that converts spoken number words to digit form.
///
/// Examples (non-exhaustive):
/// - `"twenty three items"` → `"23 items"`
/// - `"one hundred and fifty dollars"` → `"150 dollars"`
/// - `"three point five"` → `"3.5"`
/// - `"twenty first floor"` → `"21st floor"`
/// - `"negative twenty"` → `"-20"`
/// - `"a thousand users"` → `"1000 users"`
///
/// Standalone ordinals (first, second, third …) are intentionally left
/// unchanged — they are too context-dependent to convert safely
/// ("give me a second", "second opinion").
/// Hyphenated forms ("twenty-three") are not currently handled.
pub fn convert_number_words(text: &str) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return text.to_string();
    }

    let mut result: Vec<String> = Vec::new();
    let mut i = 0;

    while i < words.len() {
        if let Some((num_str, consumed)) = try_parse_number(&words, i) {
            result.push(num_str);
            i += consumed;
        } else {
            result.push(words[i].to_string());
            i += 1;
        }
    }

    result.join(" ")
}

/// Applies exact correction pairs to transcribed text.
///
/// Each pair maps a commonly mis-transcribed string to its intended replacement.
/// Matching is case-insensitive and respects word boundaries (alphanumeric edges),
/// so "aws" won't corrupt "awesome". Pairs are applied in order.
pub fn apply_correction_pairs(text: &str, pairs: &[CorrectionPair]) -> String {
    if pairs.is_empty() {
        return text.to_string();
    }

    let mut result = text.to_string();
    for pair in pairs {
        if pair.from.is_empty() {
            continue;
        }
        let escaped = regex::escape(&pair.from);
        // \b word boundaries work at string edges and around punctuation/spaces.
        // Fall back to no-boundary match if the from string starts/ends with a
        // non-word character (e.g. "#tag"), which makes \b invalid at that edge.
        let pattern = if pair.from.chars().next().map_or(false, |c| c.is_alphanumeric())
            && pair.from.chars().last().map_or(false, |c| c.is_alphanumeric())
        {
            format!(r"(?i)\b{}\b", escaped)
        } else {
            format!(r"(?i){}", escaped)
        };
        if let Ok(re) = Regex::new(&pattern) {
            result = re.replace_all(&result, pair.to.as_str()).into_owned();
        }
    }
    result
}
