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
        .map(|w| build_match_key(w))
        .collect::<Vec<_>>()
        .concat()
}

fn build_match_key(word: &str) -> String {
    word.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

struct CustomWordMatchKey {
    word_index: usize,
    key: String,
}

fn build_custom_word_match_keys(word: &str, word_index: usize) -> Vec<CustomWordMatchKey> {
    let primary_key = build_match_key(word);
    let mut keys = Vec::with_capacity(2);

    // The fallback matcher is intentionally limited to ASCII terms. Its
    // whitespace tokenization and Soundex scoring are not suitable for CJK
    // scripts. Unicode custom words remain available to models that accept
    // them as native decode prompts; they are simply skipped by this fallback.
    if is_supported_fuzzy_key(&primary_key) {
        keys.push(CustomWordMatchKey {
            word_index,
            key: primary_key.clone(),
        });
    }

    if word.contains('&') {
        let expanded_key = build_match_key(&word.replace('&', " and "));
        if is_supported_fuzzy_key(&expanded_key) && expanded_key != primary_key {
            keys.push(CustomWordMatchKey {
                word_index,
                key: expanded_key,
            });
        }
    }

    keys
}

fn is_supported_fuzzy_key(key: &str) -> bool {
    !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric())
}

fn supports_soundex(key: &str) -> bool {
    !key.is_empty() && key.chars().all(|c| c.is_ascii_alphabetic())
}

/// Finds the best matching custom word for a candidate string
///
/// Uses Levenshtein distance and Soundex phonetic matching to find
/// the best match above the given threshold.
///
/// # Arguments
/// * `candidate` - The cleaned/lowercased candidate string to match
/// * `custom_words` - Original custom words (for returning the replacement)
/// * `custom_word_match_keys` - Normalized custom-word keys for comparison
/// * `threshold` - Maximum similarity score to accept
///
/// # Returns
/// The best matching custom word and its score, if any match was found
fn find_best_match<'a>(
    candidate: &str,
    custom_words: &'a [String],
    custom_word_match_keys: &[CustomWordMatchKey],
    threshold: f64,
) -> Option<(&'a String, f64)> {
    if !is_supported_fuzzy_key(candidate) || candidate.chars().count() > 50 {
        return None;
    }

    let mut best_match: Option<&String> = None;
    let mut best_score = f64::MAX;

    for custom_word_key in custom_word_match_keys {
        // Skip if lengths are too different (optimization + prevents over-matching)
        // Use percentage-based check: max 25% length difference (prevents n-grams from
        // matching significantly shorter custom words, e.g., "openaigpt" vs "openai")
        let candidate_len = candidate.chars().count();
        let custom_word_len = custom_word_key.key.chars().count();
        let len_diff = candidate_len.abs_diff(custom_word_len) as f64;
        let max_len = candidate_len.max(custom_word_len) as f64;
        let max_allowed_diff = (max_len * 0.25).max(2.0); // At least 2 chars difference allowed
        if len_diff > max_allowed_diff {
            continue;
        }

        // Calculate Levenshtein distance (normalized by length)
        let levenshtein_dist = levenshtein(candidate, &custom_word_key.key);
        let levenshtein_score = if max_len > 0.0 {
            levenshtein_dist as f64 / max_len
        } else {
            1.0
        };

        // Soundex is an English/ASCII phonetic algorithm. Numeric terms can
        // still use edit distance, but must not receive a phonetic boost.
        let phonetic_match = supports_soundex(candidate)
            && supports_soundex(&custom_word_key.key)
            && soundex(candidate, &custom_word_key.key);

        // Combine scores: favor phonetic matches, but also consider string similarity
        let combined_score = if phonetic_match {
            levenshtein_score * 0.3 // Give significant boost to phonetic matches
        } else {
            levenshtein_score
        };

        // Accept if the score is good enough (configurable threshold)
        if combined_score < threshold && combined_score < best_score {
            best_match = Some(&custom_words[custom_word_key.word_index]);
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

    // Pre-compute normalized comparison keys to avoid repeated allocations.
    let custom_word_match_keys: Vec<CustomWordMatchKey> = custom_words
        .iter()
        .enumerate()
        .flat_map(|(index, word)| build_custom_word_match_keys(word, index))
        .collect();

    let words: Vec<&str> = text.split_whitespace().collect();
    let mut result = Vec::new();
    let mut i = 0;

    while i < words.len() {
        let mut best_match: Option<(usize, &String, f64)> = None;

        // Consider n-grams up to three words and choose the closest match. A
        // longest-first match can consume a following ordinary word when both
        // candidates happen to share a Soundex code (for example,
        // "Charge B, che" matching "ChargeBee").
        for n in (1..=3).rev() {
            if i + n > words.len() {
                continue;
            }

            let ngram_words = &words[i..i + n];
            // Do not consume across a punctuation boundary. In
            // "Charge B, che", the comma closes the candidate at "B,".
            if ngram_words[..n.saturating_sub(1)]
                .iter()
                .any(|word| !extract_punctuation(word).1.is_empty())
            {
                continue;
            }
            let ngram = build_ngram(ngram_words);

            if let Some((replacement, score)) =
                find_best_match(&ngram, custom_words, &custom_word_match_keys, threshold)
            {
                let is_better = best_match
                    .as_ref()
                    .is_none_or(|(_, _, best_score)| score < *best_score);
                if is_better {
                    best_match = Some((n, replacement, score));
                }
            }
        }

        if let Some((n, replacement, _)) = best_match {
            let ngram_words = &words[i..i + n];
            // Extract punctuation from first and last words of the n-gram.
            let (prefix, _) = extract_punctuation(ngram_words[0]);
            let (_, suffix) = extract_punctuation(ngram_words[n - 1]);

            // Preserve case from first word.
            let corrected = preserve_case_pattern(ngram_words[0], replacement);

            result.push(format!("{}{}{}", prefix, corrected, suffix));
            i += n;
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
        let pattern = if pair
            .from
            .chars()
            .next()
            .is_some_and(|c| c.is_alphanumeric())
            && pair
                .from
                .chars()
                .last()
                .is_some_and(|c| c.is_alphanumeric())
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

/// Preserves the case pattern of the original word when applying a replacement
fn preserve_case_pattern(original: &str, replacement: &str) -> String {
    if original.chars().all(|c| c.is_uppercase()) {
        replacement.to_uppercase()
    } else if original.chars().next().is_some_and(|c| c.is_uppercase()) {
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
    // String slices use byte offsets. Derive both boundaries from char_indices
    // so multibyte punctuation such as `。` and `「」` can never be split.
    let prefix_end = word
        .char_indices()
        .find(|(_, c)| c.is_alphanumeric())
        .map(|(index, _)| index)
        .unwrap_or(word.len());
    let suffix_start = word
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_alphanumeric())
        .map(|(index, c)| index + c.len_utf8())
        .unwrap_or(0);

    let prefix = if prefix_end > 0 {
        &word[..prefix_end]
    } else {
        ""
    };

    let suffix = if suffix_start < word.len() {
        &word[suffix_start..]
    } else {
        ""
    };

    (prefix, suffix)
}

/// Evidence for the language of the text being cleaned.
///
/// This intentionally describes the transcription output, not Handy's UI
/// language. Unknown output languages fail closed: built-in filler removal is
/// skipped rather than applying a language profile speculatively.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OutputLanguageEvidence {
    UserSelected(String),
    ModelConstrained(String),
    /// The transcription model itself identified the language (audio-based
    /// LID, e.g. Whisper in auto mode).
    ModelDetected(String),
    /// Detected from the transcribed text with high confidence, constrained to
    /// the model's supported languages. Weakest accepted evidence.
    TextDetected(String),
    TranslatedToEnglish,
    Unknown,
}

impl OutputLanguageEvidence {
    fn language(&self) -> Option<&str> {
        match self {
            Self::UserSelected(language)
            | Self::ModelConstrained(language)
            | Self::ModelDetected(language)
            | Self::TextDetected(language) => Some(language),
            Self::TranslatedToEnglish => Some("en"),
            Self::Unknown => None,
        }
    }
}

/// Filler tokens that are not lexical words in any language Handy's models can
/// output, so removing them cannot corrupt text regardless of the (possibly
/// unknown) output language. Kept deliberately conservative: anything that is a
/// real word somewhere ("um" pt/de, "ha" es, "ah"/"eh" interjections, "mm"
/// millimetres) belongs in the language-gated lists instead.
const UNIVERSAL_FILLER_WORDS: &[&str] = &[
    "uh", "uhm", "umm", "uhh", "uhhh", "ehh", "ehm", "ahm", "hmm", "hm", "mmm", "хм", "ммм",
];

/// Filler words that are only safe to remove with evidence for the output
/// language, because the same token is a real word elsewhere (e.g. Portuguese
/// "um" = "a/an", German "um" = "at/around", Spanish "ha" = "has").
fn gated_filler_words_for_language(lang: &str) -> &'static [&'static str] {
    let base_lang = lang.split(&['-', '_'][..]).next().unwrap_or(lang);

    match base_lang {
        "en" => &["um", "ah", "eh", "ha"],
        "de" => &["äh", "ähm"],
        "fr" => &["euh"],
        _ => &[],
    }
}

static MULTI_SPACE_PATTERN: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s{2,}").unwrap());

/// Collapses repeated words (3+ repetitions) to a single instance.
/// E.g., "wh wh wh wh" -> "wh", "I I I I" -> "I"
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

        if word_lower.chars().all(|c| c.is_alphabetic()) {
            // Count consecutive repetitions (case-insensitive)
            let mut count = 1;
            while i + count < words.len() && words[i + count].to_lowercase() == word_lower {
                count += 1;
            }

            // If 3+ repetitions, collapse to single instance
            if count >= 3 {
                result.push(word);
                i += count;
            } else {
                result.push(word);
                i += 1;
            }
        } else {
            result.push(word);
            i += 1;
        }
    }

    result.join(" ")
}

/// Removes filler words from transcription output when enabled.
///
/// Built-in removal is two-tiered: [`UNIVERSAL_FILLER_WORDS`] apply regardless
/// of language evidence, while [`gated_filler_words_for_language`] tokens are
/// only removed when the output language is known. A custom list is an
/// explicit user override and replaces both tiers without requiring language
/// evidence. `Some(empty vec)` disables removal, preserving the legacy
/// power-user setting. The master toggle takes precedence over both built-in
/// and custom lists.
///
/// # Arguments
/// * `text` - The raw transcription text to filter
/// * `language` - Evidence for the language of the transcription output
/// * `custom_filler_words` - Optional user-provided filler word list. `Some(vec)` overrides
///   language defaults; `Some(empty vec)` disables filtering; `None` uses language defaults.
/// * `enabled` - Whether filler-word removal is enabled
///
/// # Returns
/// The text with configured filler words removed
pub fn remove_filler_words(
    text: &str,
    language: &OutputLanguageEvidence,
    custom_filler_words: &Option<Vec<String>>,
    enabled: bool,
) -> String {
    if !enabled {
        return text.to_string();
    }

    // Build filler patterns from custom list or the built-in tiers
    let patterns: Vec<Regex> = match custom_filler_words {
        Some(words) => words
            .iter()
            .filter_map(|word| Regex::new(&format!(r"(?i)\b{}\b[,.]?", regex::escape(word))).ok())
            .collect(),
        None => UNIVERSAL_FILLER_WORDS
            .iter()
            .chain(
                language
                    .language()
                    .map(gated_filler_words_for_language)
                    .unwrap_or_default(),
            )
            .map(|word| Regex::new(&format!(r"(?i)\b{}\b[,.]?", regex::escape(word))).unwrap())
            .collect(),
    };

    // Remove filler words
    let mut filtered = text.to_string();
    for pattern in &patterns {
        filtered = pattern.replace_all(&filtered, "").to_string();
    }

    filtered
}

/// Applies non-filler transcription cleanup.
///
/// Kept separate from [`remove_filler_words`] so disabling filler deletion
/// does not also disable the existing repeated-word and whitespace cleanup.
pub fn normalize_transcription_output(text: &str) -> String {
    let mut normalized = collapse_stutters(text);

    // Clean up multiple spaces to single space
    normalized = MULTI_SPACE_PATTERN
        .replace_all(&normalized, " ")
        .to_string();

    // Trim leading/trailing whitespace
    normalized.trim().to_string()
}

// ==============================================================================
// Number word → digit conversion
// ==============================================================================

/// Returns the ordinal suffix for a number (e.g. 1 → "st", 2 → "nd", 3 → "rd").
fn ordinal_suffix_for(n: u64) -> &'static str {
    match n % 100 {
        11..=13 => "th",
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
        "zero" => NumWord::Ones(0),
        "one" => NumWord::Ones(1),
        "two" => NumWord::Ones(2),
        "three" => NumWord::Ones(3),
        "four" => NumWord::Ones(4),
        "five" => NumWord::Ones(5),
        "six" => NumWord::Ones(6),
        "seven" => NumWord::Ones(7),
        "eight" => NumWord::Ones(8),
        "nine" => NumWord::Ones(9),
        "ten" => NumWord::Ones(10),
        "eleven" => NumWord::Ones(11),
        "twelve" => NumWord::Ones(12),
        "thirteen" => NumWord::Ones(13),
        "fourteen" => NumWord::Ones(14),
        "fifteen" => NumWord::Ones(15),
        "sixteen" => NumWord::Ones(16),
        "seventeen" => NumWord::Ones(17),
        "eighteen" => NumWord::Ones(18),
        "nineteen" => NumWord::Ones(19),
        "twenty" => NumWord::Tens(20),
        "thirty" => NumWord::Tens(30),
        "forty" => NumWord::Tens(40),
        "fifty" => NumWord::Tens(50),
        "sixty" => NumWord::Tens(60),
        "seventy" => NumWord::Tens(70),
        "eighty" => NumWord::Tens(80),
        "ninety" => NumWord::Tens(90),
        "hundred" => NumWord::Hundred,
        "thousand" => NumWord::BigScale(1_000),
        "million" => NumWord::BigScale(1_000_000),
        "billion" => NumWord::BigScale(1_000_000_000),
        "and" => NumWord::Connector,
        "point" => NumWord::Point,
        // Ordinals — only valid at the tail of a multi-word number sequence
        "first" => NumWord::Ordinal(1),
        "second" => NumWord::Ordinal(2),
        "third" => NumWord::Ordinal(3),
        "fourth" => NumWord::Ordinal(4),
        "fifth" => NumWord::Ordinal(5),
        "sixth" => NumWord::Ordinal(6),
        "seventh" => NumWord::Ordinal(7),
        "eighth" => NumWord::Ordinal(8),
        "ninth" => NumWord::Ordinal(9),
        "tenth" => NumWord::Ordinal(10),
        "eleventh" => NumWord::Ordinal(11),
        "twelfth" => NumWord::Ordinal(12),
        "thirteenth" => NumWord::Ordinal(13),
        "fourteenth" => NumWord::Ordinal(14),
        "fifteenth" => NumWord::Ordinal(15),
        "sixteenth" => NumWord::Ordinal(16),
        "seventeenth" => NumWord::Ordinal(17),
        "eighteenth" => NumWord::Ordinal(18),
        "nineteenth" => NumWord::Ordinal(19),
        "twentieth" => NumWord::Ordinal(20),
        "thirtieth" => NumWord::Ordinal(30),
        "fortieth" => NumWord::Ordinal(40),
        "fiftieth" => NumWord::Ordinal(50),
        "sixtieth" => NumWord::Ordinal(60),
        "seventieth" => NumWord::Ordinal(70),
        "eightieth" => NumWord::Ordinal(80),
        "ninetieth" => NumWord::Ordinal(90),
        "hundredth" => NumWord::ScaleOrdinal(100),
        "thousandth" => NumWord::ScaleOrdinal(1_000),
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
        classify_number_word(&next_lower)?;
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

    Some((
        format!("{}{}{}", lead_punct, num, trail_punct),
        total_consumed,
    ))
}

/// Words that, when immediately preceding a standalone "one", indicate it is
/// a pronoun rather than a quantity ("this one", "that one", "no one", …).
const PRONOUN_ONE_INHIBITORS: &[&str] = &["this", "that", "which", "no", "each", "every"];

/// Returns true when the word at `pos` is a standalone "one" that follows a
/// demonstrative or similar word, making it a pronoun rather than a numeral.
fn is_pronoun_one(words: &[&str], pos: usize, consumed: usize) -> bool {
    if consumed != 1 || word_core(words[pos]).to_lowercase() != "one" || pos == 0 {
        return false;
    }
    let prev = word_core(words[pos - 1]).to_lowercase();
    PRONOUN_ONE_INHIBITORS.contains(&prev.as_str())
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
///
/// Pronoun "one" after demonstratives ("this one", "that one", "which one",
/// "no one", "each one", "every one") is also left unchanged.
pub fn convert_number_words(text: &str) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return text.to_string();
    }

    let mut result: Vec<String> = Vec::new();
    let mut i = 0;

    while i < words.len() {
        if let Some((num_str, consumed)) = try_parse_number(&words, i) {
            if is_pronoun_one(&words, i, consumed) {
                result.push(words[i].to_string());
            } else {
                result.push(num_str);
            }
            i += consumed;
        } else {
            result.push(words[i].to_string());
            i += 1;
        }
    }

    result.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exercise the complete cleanup sequence with an explicitly selected
    /// language. Individual tests below predate the split between filler
    /// removal and non-filler normalization.
    fn filter_transcription_output(
        text: &str,
        language: &str,
        custom_filler_words: &Option<Vec<String>>,
    ) -> String {
        let language = OutputLanguageEvidence::UserSelected(language.to_string());
        let filtered = remove_filler_words(text, &language, custom_filler_words, true);
        normalize_transcription_output(&filtered)
    }

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
    fn test_apply_correction_pairs_empty() {
        let text = "hello world";
        let result = apply_correction_pairs(text, &[]);
        assert_eq!(result, "hello world");
    }

    #[test]
    fn test_apply_correction_pairs_basic_replacement() {
        let text = "I use aws for cloud hosting";
        let pairs = vec![CorrectionPair {
            from: "aws".to_string(),
            to: "AWS".to_string(),
        }];
        let result = apply_correction_pairs(text, &pairs);
        assert_eq!(result, "I use AWS for cloud hosting");
    }

    #[test]
    fn test_apply_correction_pairs_case_insensitive_match() {
        let text = "AWS and Aws and aws";
        let pairs = vec![CorrectionPair {
            from: "aws".to_string(),
            to: "AWS".to_string(),
        }];
        let result = apply_correction_pairs(text, &pairs);
        assert_eq!(result, "AWS and AWS and AWS");
    }

    #[test]
    fn test_apply_correction_pairs_respects_word_boundaries() {
        let text = "that was awesome, not aws";
        let pairs = vec![CorrectionPair {
            from: "aws".to_string(),
            to: "AWS".to_string(),
        }];
        let result = apply_correction_pairs(text, &pairs);
        assert_eq!(result, "that was awesome, not AWS");
    }

    #[test]
    fn test_apply_correction_pairs_multiple_pairs_applied_in_order() {
        let text = "chat gpt and claude ai";
        let pairs = vec![
            CorrectionPair {
                from: "chat gpt".to_string(),
                to: "ChatGPT".to_string(),
            },
            CorrectionPair {
                from: "claude ai".to_string(),
                to: "Claude AI".to_string(),
            },
        ];
        let result = apply_correction_pairs(text, &pairs);
        assert_eq!(result, "ChatGPT and Claude AI");
    }

    #[test]
    fn test_apply_correction_pairs_skips_empty_from() {
        let text = "hello world";
        let pairs = vec![CorrectionPair {
            from: "".to_string(),
            to: "x".to_string(),
        }];
        let result = apply_correction_pairs(text, &pairs);
        assert_eq!(result, "hello world");
    }

    #[test]
    fn test_apply_correction_pairs_non_alphanumeric_edges_no_word_boundary() {
        let text = "check out #tag today";
        let pairs = vec![CorrectionPair {
            from: "#tag".to_string(),
            to: "#hashtag".to_string(),
        }];
        let result = apply_correction_pairs(text, &pairs);
        assert_eq!(result, "check out #hashtag today");
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
    fn test_extract_punctuation_uses_unicode_boundaries() {
        assert_eq!(extract_punctuation("你好。"), ("", "。"));
        assert_eq!(extract_punctuation("「你好」"), ("「", "」"));
        assert_eq!(extract_punctuation("你好！"), ("", "！"));
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
        let text = "w wh wh wh wh wh wh wh wh wh why";
        let result = filter_transcription_output(text, "en", &None);
        assert_eq!(result, "w wh why");
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
    fn test_filter_unknown_language_still_removes_universal_fillers() {
        let text = "uh I think uhm this works";
        let result = filter_transcription_output(text, "xx", &None);
        assert_eq!(result, "I think this works");
    }

    #[test]
    fn test_filter_unknown_language_does_not_remove_um() {
        let text = "um I think this works";
        let result = filter_transcription_output(text, "xx", &None);
        assert_eq!(result, "um I think this works");
    }

    #[test]
    fn test_filter_unknown_evidence_removes_universal_keeps_gated() {
        let filtered = remove_filler_words(
            "uhh bueno hmm creo que um ha llegado",
            &OutputLanguageEvidence::Unknown,
            &None,
            true,
        );
        assert_eq!(
            normalize_transcription_output(&filtered),
            "bueno creo que um ha llegado"
        );

        let cyrillic = remove_filler_words(
            "хм я думаю ммм это работает",
            &OutputLanguageEvidence::Unknown,
            &None,
            true,
        );
        assert_eq!(
            normalize_transcription_output(&cyrillic),
            "я думаю это работает"
        );
    }

    #[test]
    fn test_filter_german_gated_fillers_require_evidence() {
        let text = "äh ich glaube ähm das passt";

        let unknown = remove_filler_words(text, &OutputLanguageEvidence::Unknown, &None, true);
        assert_eq!(normalize_transcription_output(&unknown), text);

        let result = filter_transcription_output(text, "de", &None);
        assert_eq!(result, "ich glaube das passt");
    }

    #[test]
    fn test_filter_preserves_millimetre_unit() {
        // "mm" was removed from the filler lists because it eats units.
        let text = "the screw is 5 mm long";
        let result = filter_transcription_output(text, "en", &None);
        assert_eq!(result, "the screw is 5 mm long");
    }

    #[test]
    fn test_filter_detected_evidence_unlocks_gated_fillers() {
        let model = remove_filler_words(
            "um I think this works",
            &OutputLanguageEvidence::ModelDetected("en".to_string()),
            &None,
            true,
        );
        assert_eq!(normalize_transcription_output(&model), "I think this works");

        let text = remove_filler_words(
            "euh je pense que ça marche",
            &OutputLanguageEvidence::TextDetected("fr".to_string()),
            &None,
            true,
        );
        assert_eq!(
            normalize_transcription_output(&text),
            "je pense que ça marche"
        );
    }

    #[test]
    fn test_filter_master_toggle_disables_custom_and_builtin_removal() {
        let text = "um customword I think";
        let language = OutputLanguageEvidence::UserSelected("en".to_string());
        let custom = Some(vec!["customword".to_string()]);

        let result = remove_filler_words(text, &language, &custom, false);

        assert_eq!(result, text);
    }

    #[test]
    fn test_filter_custom_words_apply_without_language_evidence() {
        let custom = Some(vec!["customword".to_string()]);
        let text = "customword should be removed but um should remain";

        let filtered = remove_filler_words(text, &OutputLanguageEvidence::Unknown, &custom, true);
        let result = normalize_transcription_output(&filtered);

        assert_eq!(result, "should be removed but um should remain");
    }

    #[test]
    fn test_apply_custom_words_ngram_two_words() {
        let text = "il cui nome è Charge B, che permette";
        let custom_words = vec!["ChargeBee".to_string()];
        let result = apply_custom_words(text, &custom_words, 0.5);
        assert!(result.contains("ChargeBee,"), "unexpected result: {result}");
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
        assert_eq!(result, "using MacBook Pro");
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

    #[test]
    fn test_apply_custom_words_matches_ampersand_word() {
        let text = "send it to RD for review";
        let custom_words = vec!["R&D".to_string()];
        let result = apply_custom_words(text, &custom_words, 0.18);
        assert_eq!(result, "send it to R&D for review");
    }

    #[test]
    fn test_apply_custom_words_matches_spoken_ampersand_word() {
        let text = "send it to R and D for review";
        let custom_words = vec!["R&D".to_string()];
        let result = apply_custom_words(text, &custom_words, 0.18);
        assert_eq!(result, "send it to R&D for review");
    }

    #[test]
    fn test_apply_custom_words_preserves_ampersand_word() {
        let text = "send it to R&D for review";
        let custom_words = vec!["R&D".to_string()];
        let result = apply_custom_words(text, &custom_words, 0.18);
        assert_eq!(result, "send it to R&D for review");
    }

    #[test]
    fn test_apply_custom_words_handles_unicode_punctuation() {
        let text = "「Handee。」";
        let custom_words = vec!["Handy".to_string()];
        let result = apply_custom_words(text, &custom_words, 0.5);
        assert_eq!(result, "「Handy。」");
    }

    #[test]
    fn test_apply_custom_words_skips_cjk_fuzzy_matching() {
        let text = "你好。";
        let custom_words = vec!["你号".to_string()];
        let result = apply_custom_words(text, &custom_words, 1.0);
        assert_eq!(result, text);
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
        assert_eq!(
            convert_number_words("twenty three thousand four hundred and fifty six"),
            "23456"
        );
        assert_eq!(convert_number_words("one million"), "1000000");
        assert_eq!(convert_number_words("two hundred thousand"), "200000");
    }

    #[test]
    fn test_number_decimals() {
        assert_eq!(convert_number_words("three point five"), "3.5");
        assert_eq!(convert_number_words("thirty two point seven five"), "32.75");
        assert_eq!(convert_number_words("one point two three"), "1.23");
        // "point" alone (no following digit) stays unconsumed
        assert_eq!(
            convert_number_words("make a point about this"),
            "make a point about this"
        );
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
        assert_eq!(
            convert_number_words("first and foremost"),
            "first and foremost"
        );
        assert_eq!(
            convert_number_words("the second opinion"),
            "the second opinion"
        );
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
        assert_eq!(
            convert_number_words("million dollar idea"),
            "million dollar idea"
        );
    }

    #[test]
    fn test_number_pronoun_one_unchanged() {
        // Demonstratives — "one" is a pronoun, not a quantity
        assert_eq!(convert_number_words("this one"), "this one");
        assert_eq!(convert_number_words("that one"), "that one");
        assert_eq!(convert_number_words("which one"), "which one");
        assert_eq!(convert_number_words("no one"), "no one");
        assert_eq!(convert_number_words("each one"), "each one");
        assert_eq!(convert_number_words("every one"), "every one");
        // Multi-word: only the pronoun "one" is suppressed; other numbers convert normally
        assert_eq!(
            convert_number_words("pick this one or that one out of twenty"),
            "pick this one or that one out of 20"
        );
        // "this one hundred" — "one hundred" is a multi-word quantity, consumed==2, must convert
        assert_eq!(
            convert_number_words("this one hundred users"),
            "this 100 users"
        );
        // Standalone "one" without an inhibiting predecessor still converts
        assert_eq!(convert_number_words("one item"), "1 item");
        assert_eq!(convert_number_words("give me one"), "give me 1");
    }
}
