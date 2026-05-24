use regex::Regex;

/// Replaces verbal quote delimiters with typographic quotation marks.
///
/// Recognised open markers:  "quote", "open quote", "open quotes", "begin quote"
/// Recognised close markers: "end quote", "close quote", "close quotes", "unquote"
///
/// Optional commas between the marker and the quoted content are stripped.
pub fn apply_quote_cues(text: &str) -> String {
    let re = Regex::new(
        r"(?i)\b(?:open\s+quotes?|begin\s+quote|quote)\b,?\s*(.*?)\s*,?\s*\b(?:end\s+quote|close\s+quotes?|unquote)\b",
    )
    .expect("quote_cues regex is valid");

    re.replace_all(text, "\"$1\"").into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_quote_end_quote() {
        assert_eq!(
            apply_quote_cues("The card says quote your usage is up 255 percent end quote."),
            "The card says \"your usage is up 255 percent\"."
        );
    }

    #[test]
    fn replaces_open_quote_close_quote() {
        assert_eq!(
            apply_quote_cues("He said open quote hello there close quote and left."),
            "He said \"hello there\" and left."
        );
    }

    #[test]
    fn handles_comma_after_cue() {
        assert_eq!(
            apply_quote_cues("For example, quote, this is a test, end quote."),
            "For example, \"this is a test\"."
        );
    }

    #[test]
    fn handles_unquote() {
        assert_eq!(
            apply_quote_cues("She said quote be careful unquote."),
            "She said \"be careful\"."
        );
    }

    #[test]
    fn handles_open_quotes_plural() {
        assert_eq!(
            apply_quote_cues("open quotes hello world close quotes"),
            "\"hello world\""
        );
    }

    #[test]
    fn no_match_leaves_text_unchanged() {
        let input = "This has no quote markers at all.";
        assert_eq!(apply_quote_cues(input), input);
    }

    #[test]
    fn case_insensitive() {
        assert_eq!(
            apply_quote_cues("Quote hello world End Quote"),
            "\"hello world\""
        );
    }

    #[test]
    fn isolated_quote_verb_not_affected() {
        // "quote" as a verb with no matching close marker must not be changed
        let input = "I want to quote Shakespeare here.";
        assert_eq!(apply_quote_cues(input), input);
    }
}
