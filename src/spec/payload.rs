//! The grammars of the payload data types that are not dates, ages or
//! times (those come from [`crate::types`]), and the characters a payload
//! may hold.
//!
//! Each check returns `None` when the payload is valid, else why it is not.

use std::borrow::Cow;

use crate::types::age::AgeValue;
use crate::types::date::{DateExact, DatePeriod, DateValue, Time};
use crate::version::VersionRules;
use crate::writer::{special_bytes, CONTROL, MAYBE_BANNED, TAB};
use crate::GedcomVersion;

use super::schema::{EnumSet, Kind};

/// The grammar family of a version: 7.0 and 7.1 share theirs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Family {
    V551,
    V7,
}

impl Family {
    /// The family of a version's rules.
    pub(crate) fn of(rules: &VersionRules) -> Family {
        if rules.version.is_v7() {
            Family::V7
        } else {
            Family::V551
        }
    }

    /// The version whose grammars the family follows.
    pub(crate) fn version(self) -> GedcomVersion {
        match self {
            Family::V551 => GedcomVersion::V5_5_1,
            Family::V7 => GedcomVersion::V7_0,
        }
    }
}

/// Whether a character may not appear in a payload of the tree
/// ([`VersionRules::is_banned`]). Line breaks inside text are written as
/// `CONT` lines, so they are never banned here.
#[inline]
pub(crate) fn is_banned(c: char, rules: &VersionRules) -> bool {
    !matches!(c, '\n' | '\r') && rules.is_banned(c)
}

/// Whether `text` holds a character [`is_banned`] bans. Only a C0 control
/// but a line break, DEL, or the lead bytes of a C1 control (`C2`) or of
/// U+FFFE and U+FFFF (`EF`) can start one: text without them, nearly all
/// of it, is tested eight bytes at a time ([`special_bytes`]).
#[inline]
pub(crate) fn has_banned(text: &str, rules: &VersionRules) -> bool {
    has_banned_known(text, special_bytes(text), rules)
}

/// [`has_banned`] of a text whose [`special_bytes`] are `special`.
#[inline]
pub(crate) fn has_banned_known(text: &str, special: u8, rules: &VersionRules) -> bool {
    special & (TAB | CONTROL | MAYBE_BANNED) != 0 && text.chars().any(|c| is_banned(c, rules))
}

/// Whether `text` is a valid payload of `kind` ([`check`] finds nothing).
#[inline]
pub(crate) fn is_valid(kind: Kind, set: Option<&EnumSet>, text: &str, family: Family) -> bool {
    let simple = match kind {
        // 5.5.1 reads month names in any case (p. 21).
        Kind::Date => is_simple_date(text, false, family == Family::V551),
        Kind::DateExact => is_simple_date(text, true, family == Family::V551),
        _ => false,
    };
    if simple {
        debug_assert_eq!(check(kind, set, text, family), None, "{text:?}");
        return true;
    }
    check(kind, set, text, family).is_none()
}

/// Whether `text` is a Gregorian date of the commonest shapes, valid in
/// both versions without a full parse: `[ABT|CAL|EST|BEF|AFT ]` then
/// `YYYY`, `MON YYYY` or `D MON YYYY` (only the last, without a keyword,
/// when `exact`), single spaces, keywords and months in upper case (months
/// in any case when `any_case`, valid in 5.5.1 only), a year of one to four
/// digits from 1, a day of the month from 1. Anything else is left to the
/// grammar.
pub(crate) fn is_simple_date(text: &str, exact: bool, any_case: bool) -> bool {
    // A number of one to `max_len` digits, not starting with 0.
    let number = |w: &[u8], max_len: usize| -> Option<u16> {
        if w.is_empty() || w.len() > max_len || w.first() == Some(&b'0') {
            return None;
        }
        w.iter().try_fold(0_u16, |n, &c| {
            c.is_ascii_digit().then(|| n * 10 + u16::from(c - b'0'))
        })
    };
    let leap = |y: u16| y.is_multiple_of(4) && (!y.is_multiple_of(100) || y.is_multiple_of(400));
    let days = |m: &[u8], y: u16| -> Option<u16> {
        let upper = match *m {
            [x, y, z] => [x, y, z].map(|c| c.to_ascii_uppercase()),
            _ => return None,
        };
        if !any_case && upper != m {
            return None;
        }
        Some(match &upper {
            b"JAN" | b"MAR" | b"MAY" | b"JUL" | b"AUG" | b"OCT" | b"DEC" => 31,
            b"APR" | b"JUN" | b"SEP" | b"NOV" => 30,
            b"FEB" if leap(y) => 29,
            b"FEB" => 28,
            _ => return None,
        })
    };
    let mut words = text.as_bytes().split(|&c| c == b' ');
    let mut first = words.next();
    if !exact && matches!(first, Some(b"ABT" | b"CAL" | b"EST" | b"BEF" | b"AFT")) {
        first = words.next();
    }
    match (first, words.next(), words.next(), words.next()) {
        (Some(y), None, None, None) => !exact && number(y, 4).is_some(),
        (Some(m), Some(y), None, None) => {
            !exact && number(y, 4).is_some_and(|y| days(m, y).is_some())
        }
        (Some(d), Some(m), Some(y), None) => number(y, 4).is_some_and(|y| {
            number(d, 2)
                .zip(days(m, y))
                .is_some_and(|(d, max)| d <= max)
        }),
        _ => false,
    }
}

/// Why `text` is not a valid payload of `kind` (an enumeration of `set`).
/// Pointers are checked by the caller.
pub(crate) fn check(
    kind: Kind,
    set: Option<&EnumSet>,
    text: &str,
    family: Family,
) -> Option<String> {
    let v = family.version();
    match kind {
        Kind::None => (!text.is_empty()).then(|| "this structure takes no payload".into()),
        Kind::Y => (!(text.is_empty() || text == "Y")).then(|| "only `Y` or nothing".into()),
        // Pointers are checked by the caller.
        Kind::Text | Kind::ListText | Kind::Pointer | Kind::NullablePointer => None,
        Kind::Int => (!is_digits(text)).then(|| "not a non-negative integer".into()),
        Kind::Name => name(text),
        Kind::Date => DateValue::parse_strict(text, v).err().map(|e| e.reason),
        Kind::DateExact => DateExact::parse_strict(text, v).err().map(|e| e.reason),
        Kind::DatePeriod => DatePeriod::parse_strict(text, v).err().map(|e| e.reason),
        Kind::Age => {
            // 5.5.1 shows the bound and the age with a space between them,
            // as its patterns show every part (p. 42); both spellings are
            // read as written.
            let text = match (family, text.as_bytes()) {
                (Family::V551, [b'<' | b'>', b' ', ..]) => {
                    Cow::Owned(format!("{}{}", &text[..1], &text[2..]))
                }
                _ => Cow::Borrowed(text),
            };
            AgeValue::parse_strict(&text, v).err().map(|e| e.reason)
        }
        Kind::Time => Time::parse_strict(text, v).err().map(|e| e.reason),
        // 5.5.1 languages are an enumeration; this kind is 7.x only.
        Kind::Lang => (!is_language_tag(text)).then(|| "not a BCP 47 language tag".into()),
        Kind::MediaType => (!is_media_type(text)).then(|| "not a media type".into()),
        Kind::FilePath => file_path(text, family),
        Kind::Uri => (!is_absolute_uri(text)).then(|| "not an absolute URI".into()),
        Kind::Lat => (!is_coordinate(text, ['N', 'S'], 90)).then(|| "not a latitude".into()),
        Kind::Long => (!is_coordinate(text, ['E', 'W'], 180)).then(|| "not a longitude".into()),
        Kind::TagDef => (!is_tag_def(text)).then(|| "not an extension tag and its URI".into()),
        Kind::Enum => set.and_then(|s| enum_value(s, text, family)),
        Kind::ListEnum => set.and_then(|s| {
            list_items(text)
                .find_map(|item| enum_value(s, item, family))
                .or_else(|| {
                    text.is_empty()
                        .then(|| format!("not a value of {}", s.name))
                })
        }),
    }
}

/// The items of a list payload (7.x `List`: items separated by commas and
/// any spaces).
pub(crate) fn list_items(text: &str) -> impl Iterator<Item = &str> {
    text.split(',').map(|i| i.trim_matches(' '))
}

/// Why `value` is not a value of `set`. 5.5.1 values are matched without
/// regard to case (p. 21); 7.x values are exact, and any extension tag is a
/// value of every set (§1.5.2).
pub(crate) fn enum_value(set: &EnumSet, value: &str, family: Family) -> Option<String> {
    if set.open && !value.is_empty() {
        return None;
    }
    let ok = match family {
        Family::V551 => set.values.iter().any(|v| v.eq_ignore_ascii_case(value)),
        Family::V7 => set.values.contains(&value) || is_ext_tag(value),
    };
    (!ok).then(|| format!("not a value of {}", set.name))
}

/// Whether `s` is one or more ASCII digits.
pub(crate) fn is_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// A 7.x extension tag: `_` and one or more `A`–`Z`, digits or `_`.
pub(crate) fn is_ext_tag(s: &str) -> bool {
    s.strip_prefix('_').is_some_and(|rest| {
        !rest.is_empty()
            && rest
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
    })
}

/// 7.x `PersonalName` and 5.5.1 `NAME_PERSONAL`: no control character,
/// and either no slash or one pair of slashes around the surname.
fn name(text: &str) -> Option<String> {
    if text.chars().any(char::is_control) {
        return Some("a name holds no control character".into());
    }
    match text.matches('/').count() {
        0 | 2 => None,
        _ => Some("the surname takes one pair of slashes".into()),
    }
}

/// BCP 47 syntax: a primary subtag of 2 to 8 letters (or `x`/`i` for
/// private-use and grandfathered tags), then subtags of 1 to 8 letters or
/// digits, separated by hyphens.
fn is_language_tag(s: &str) -> bool {
    let mut parts = s.split('-');
    let first = parts.next().unwrap_or("");
    let primary = (2..=8).contains(&first.len()) && first.bytes().all(|b| b.is_ascii_alphabetic())
        || matches!(first, "x" | "X" | "i" | "I");
    let mut rest = parts.peekable();
    primary
        && (rest.peek().is_some() || first.len() > 1)
        && rest.all(|p| (1..=8).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_alphanumeric()))
}

/// RFC 6838 `type "/" subtype` with optional `; name=value` parameters.
fn is_media_type(s: &str) -> bool {
    let restricted = |t: &str| {
        let b = t.as_bytes();
        (1..=127).contains(&b.len())
            && b.first().is_some_and(u8::is_ascii_alphanumeric)
            && b.iter()
                .all(|c| c.is_ascii_alphanumeric() || b"!#$&-^_.+".contains(c))
    };
    let mut parts = s.split(';');
    let main = parts.next().unwrap_or("");
    let main_ok = main
        .split_once('/')
        .is_some_and(|(t, sub)| restricted(t) && restricted(sub));
    main_ok
        && parts.all(|p| {
            p.trim_start_matches(' ')
                .split_once('=')
                .is_some_and(|(n, v)| restricted(n) && !v.is_empty() && !v.contains(' '))
        })
}

/// Whether byte `c` may appear in a URI as itself (RFC 3986 unreserved,
/// sub-delims, `:`, `@`, `/`, `?`, `#`, `[`, `]`), as part of a
/// percent-encoding, or as part of a non-ASCII character, which a WHATWG URL
/// string admits (7.x §2.12).
fn is_uri_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || b"-._~!$&'()*+,;=:@/?#[]%".contains(&c) || c >= 0x80
}

/// The scheme of a URI reference, when it has one (RFC 3986 §3.1).
fn scheme(s: &str) -> Option<&str> {
    let (scheme, _) = s.split_once(':')?;
    let b = scheme.as_bytes();
    (b.first().is_some_and(u8::is_ascii_alphabetic)
        && b.iter()
            .all(|c| c.is_ascii_alphanumeric() || b"+-.".contains(c)))
    .then_some(scheme)
}

/// Whether every `%` starts a percent-encoding and every character is a
/// URI character.
fn uri_chars_ok(s: &str) -> bool {
    let b = s.as_bytes();
    b.iter().all(|&c| is_uri_char(c))
        && b.iter().enumerate().all(|(i, &c)| {
            c != b'%'
                || b.get(i + 1..i + 3)
                    .is_some_and(|h| h.iter().all(u8::is_ascii_hexdigit))
        })
}

/// RFC 3986 `absolute-URI`, loosely: a scheme, then URI characters.
fn is_absolute_uri(s: &str) -> bool {
    scheme(s).is_some() && uri_chars_ok(s)
}

/// 7.x `FilePath` (§2.12): a URI reference that is an `ftp`, `http`,
/// `https` or `file` URL, or a relative reference that does not start with
/// `/`, has no `..` segment, no backslash (escaped or not), no query and
/// no fragment. A `file` URL names a host without a port (RFC 8089).
/// 5.5.1 `MULTIMEDIA_FILE_REFERENCE` is any text.
fn file_path(s: &str, family: Family) -> Option<String> {
    if family == Family::V551 || s.is_empty() {
        return None;
    }
    if !uri_chars_ok(s) {
        return Some("not a URI reference".into());
    }
    if let Some(scheme) = scheme(s) {
        let scheme = scheme.to_ascii_lowercase();
        if !matches!(scheme.as_str(), "ftp" | "http" | "https" | "file") {
            return Some(format!("the {scheme} scheme is not supported"));
        }
        if scheme == "file" {
            let rest = s.get(5..).unwrap_or("");
            if let Some(auth) = rest.strip_prefix("//") {
                let host = auth.split('/').next().unwrap_or("");
                if host.contains([':', '@']) {
                    return Some("a file URL names a host and nothing else".into());
                }
            }
        }
        return None;
    }
    let lower = s.to_ascii_lowercase();
    if s.starts_with('/') {
        Some("a local file path does not start with /".into())
    } else if s.split('/').any(|seg| seg == "..") {
        Some("a local file path has no .. segment".into())
    } else if lower.contains("%5c") {
        Some("a local file path has no backslash".into())
    } else if s.contains(['?', '#']) {
        Some("a local file path has no query or fragment".into())
    } else if s.split('/').next().is_some_and(|seg| seg.contains(':')) {
        Some("not a URI reference".into())
    } else {
        None
    }
}

/// `N`/`S` (or `E`/`W`), then a decimal number of degrees up to `max`.
fn is_coordinate(s: &str, dirs: [char; 2], max: u32) -> bool {
    let Some(rest) = s.strip_prefix(dirs) else {
        return false;
    };
    let (int, frac) = rest.split_once('.').unwrap_or((rest, "0"));
    is_digits(int)
        && is_digits(frac)
        && int.len() <= 3
        && int
            .parse::<u32>()
            .is_ok_and(|d| d < max || d == max && frac.bytes().all(|b| b == b'0'))
}

/// 7.x `TagDef`: an extension tag, a space, an absolute URI.
fn is_tag_def(s: &str) -> bool {
    s.split_once(' ')
        .is_some_and(|(tag, uri)| is_ext_tag(tag) && is_absolute_uri(uri))
}

/// The payload kinds for which an empty payload is still a value.
pub(crate) fn empty_is_valid(kind: Kind) -> bool {
    !matches!(
        kind,
        Kind::Int | Kind::Enum | Kind::ListEnum | Kind::Lang | Kind::MediaType
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        for ok in ["John Smith", "John /Smith/", "John /Smith/ Jr.", "//", ""] {
            assert!(name(ok).is_none(), "{ok:?}");
        }
        for bad in ["/", "a/b/c/d", "a\tb"] {
            assert!(name(bad).is_some(), "{bad:?}");
        }
    }

    #[test]
    fn languages() {
        for ok in ["und", "mul", "en", "en-US", "und-Latn-pinyin", "x-private"] {
            assert!(is_language_tag(ok), "{ok:?}");
        }
        for bad in [" ", "-", "und-", "-und", "en US", "", "x", "toolongtag"] {
            assert!(!is_language_tag(bad), "{bad:?}");
        }
    }

    #[test]
    fn media_types() {
        for ok in [
            "application/x-other",
            "text/plain",
            "text/html; charset=utf-8",
        ] {
            assert!(is_media_type(ok), "{ok:?}");
        }
        for bad in [
            "",
            "invalid media type",
            "text/",
            "/text",
            "text/a/b",
            "text",
        ] {
            assert!(!is_media_type(bad), "{bad:?}");
        }
    }

    #[test]
    fn file_paths() {
        for ok in [
            "media/filename",
            "https://example.com/1/16/Portrait_\u{e9}t\u{e9}.jpg",
            "http://www.contoso.com/path/filename",
            "file://host.example.com/path/to/file",
            "file:///path/to/file",
            "foo",
        ] {
            assert!(file_path(ok, Family::V7).is_none(), "{ok:?}");
        }
        for bad in [
            "http://www.contoso.com/path???/file name",
            "c:\\\\directory\\filename",
            "file://c:/directory/filename",
            "http:\\\\\\host/path/file",
            "2013.05.29_14:33:41",
            "/abs/path",
            "media/../x",
            "media/a%5Cb",
            "mailto:someone@example.com",
        ] {
            assert!(file_path(bad, Family::V7).is_some(), "{bad:?}");
        }
        assert!(file_path("c:\\dir\\file", Family::V551).is_none());
    }

    #[test]
    fn coordinates() {
        assert!(is_coordinate("N18.150944", ['N', 'S'], 90));
        assert!(is_coordinate("S90", ['N', 'S'], 90));
        assert!(!is_coordinate("N90.5", ['N', 'S'], 90));
        assert!(!is_coordinate("W200", ['E', 'W'], 180));
        assert!(!is_coordinate("18.15", ['N', 'S'], 90));
    }

    #[test]
    fn banned_characters() {
        use crate::version::{V551, V70};
        assert!(is_banned('\u{7}', &V70));
        assert!(!is_banned('\t', &V70));
        assert!(is_banned('\t', &V551));
        assert!(!is_banned('\n', &V551));
        assert!(!is_banned('\r', &V70));
        assert!(is_banned('\u{85}', &V70));
        assert!(!is_banned('é', &V551));
    }

    #[test]
    fn banned_characters_by_bytes() {
        use crate::version::{V551, V70};
        for rules in [&V551, &V70] {
            for c in [
                'a', 'é', '\t', '\n', '\r', '\u{7}', '\u{7f}', '\u{85}', '\u{a0}', '\u{fffd}',
            ]
            .into_iter()
            .chain(['\u{fffe}', '\u{ffff}', '\u{10000}'])
            {
                let text = format!("ab{c}cd\nef");
                assert_eq!(
                    has_banned(&text, rules),
                    text.chars().any(|c| is_banned(c, rules)),
                    "{c:?}"
                );
            }
        }
    }

    /// The dates the fast path takes are exactly those of the grammar that
    /// it takes: every one valid in both versions.
    #[test]
    fn simple_dates_are_valid() {
        let months = [
            "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
            "Jan", "oct", "fEb", "FOO", "Foo",
        ];
        let mut taken = 0;
        for keyword in [
            "", "ABT ", "CAL ", "EST ", "BEF ", "AFT ", "abt ", "BET ", "FROM ",
        ] {
            for year in [
                "1", "4", "99", "100", "400", "1600", "1700", "1900", "1999", "2000", "2024",
                "9999", "0", "01", "10000",
            ] {
                let mut texts = vec![format!("{keyword}{year}")];
                for month in months {
                    texts.push(format!("{keyword}{month} {year}"));
                    for day in ["0", "1", "9", "01", "28", "29", "30", "31", "32"] {
                        texts.push(format!("{keyword}{day} {month} {year}"));
                        texts.push(format!("{keyword}{day}  {month} {year}"));
                    }
                }
                for text in texts {
                    for (kind, exact) in [(Kind::Date, false), (Kind::DateExact, true)] {
                        if is_simple_date(&text, exact, true) {
                            assert_eq!(
                                check(kind, None, &text, Family::V551),
                                None,
                                "{text:?} {kind:?}"
                            );
                            // Written as it is in 5.5.1, its month in upper
                            // case in 7.x.
                            let date = crate::types::date::Date {
                                value: Some(text.clone()),
                                time: None,
                                phrase: None,
                            };
                            let upper = crate::types::date::Date {
                                value: Some(text.to_ascii_uppercase()),
                                ..date.clone()
                            };
                            assert_eq!(date.to_version(GedcomVersion::V5_5_1), date, "{text:?}");
                            for version in [GedcomVersion::V7_0, GedcomVersion::V7_1] {
                                assert_eq!(date.to_version(version), upper, "{text:?} {version}");
                            }
                        }
                        if !is_simple_date(&text, exact, false) {
                            continue;
                        }
                        taken += 1;
                        for family in [Family::V551, Family::V7] {
                            assert_eq!(check(kind, None, &text, family), None, "{text:?} {kind:?}");
                        }
                        // Written alike in every version (the conversion
                        // of the typed model skips them).
                        let date = crate::types::date::Date {
                            value: Some(text.clone()),
                            time: None,
                            phrase: None,
                        };
                        for version in [
                            GedcomVersion::V5_5_1,
                            GedcomVersion::V7_0,
                            GedcomVersion::V7_1,
                        ] {
                            assert_eq!(date.to_version(version), date, "{text:?} {version}");
                        }
                    }
                }
            }
        }
        assert!(taken > 5000, "{taken}");
        assert!(!is_simple_date("29 FEB 1900", false, false));
        assert!(is_simple_date("29 FEB 2000", true, false));
        assert!(!is_simple_date("ABT 1 JAN 1900", true, false));
        assert!(!is_simple_date("2 Oct 1822", false, false));
        assert!(is_simple_date("2 Oct 1822", true, true));
    }
}
