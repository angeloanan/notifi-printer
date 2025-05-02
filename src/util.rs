use std::io::prelude::*;
use std::{io::BufWriter, sync::LazyLock};

use regex::Regex;

const PATTERN: &str = r"[\u{1F1E6}-\u{1F1FF}]{2}|\u{1F3F4}[\u{E0061}-\u{E007A}]{2}[\u{E0030}-\u{E0039}\u{E0061}-\u{E007A}]{1,3}\u{E007F}|(?:\p{Emoji}\uFE0F\u20E3?|\p{Emoji_Modifier_Base}\p{Emoji_Modifier}?|\p{Emoji_Presentation})(?:\u200D(?:\p{Emoji}\uFE0F\u20E3?|\p{Emoji_Modifier_Base}\p{Emoji_Modifier}?|\p{Emoji_Presentation}))*";
pub static EMOJI_REGEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(PATTERN).unwrap());

pub fn sanitize_emoji(string: &str) -> String {
    let mut out = String::new();

    let mut last_end = 0;
    for matches in EMOJI_REGEX.find_iter(string) {
        let matched_unicode = matches.as_str();
        let emoji = emojis::get(matched_unicode)
            .unwrap_or_else(|| panic!("Invalid emoji: {matched_unicode}"));
        let short_code = emoji.shortcode().unwrap_or_default().replace(' ', "_");
        let wrapped_short_code = format!(":{short_code}:");

        let start = matches.start();
        out.push_str(&string[last_end..start]);
        out.push_str(&wrapped_short_code);

        last_end = matches.end();
    }

    // Push last
    out.push_str(&string[last_end..]);

    out
}
