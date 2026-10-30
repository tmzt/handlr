//! Clock times read relative to when they are spoken (plan 20261023_05,
//! the 16:28 EDT amendment, A2).
//!
//! A time is `H:MM` or `H:MM:SS`, 24-hour, or 12-hour with `AM` or `PM`
//! after it; a zone may follow (`Z` attached, or `UTC`, `GMT` or a US
//! abbreviation as the next word). A time with no zone is in `now`'s zone.
//! A time is taken to be on its own zone's date today, so the difference
//! is never wrapped around midnight: 12 hours or more away, it is read as
//! the clock time in `now`'s zone. A time just after a date
//! (`2026-10-22 16:13`) may not be today, and is read as the clock time
//! too. Dates and durations are left alone.

/// The local time of day when the text is read, and its zone's offset
/// from UTC (east positive: EDT is -240).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LocalTime {
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub utc_offset_minutes: i16,
}

impl LocalTime {
    fn seconds(&self) -> i64 {
        i64::from(self.hour) * 3600 + i64::from(self.minute) * 60 + i64::from(self.second)
    }
}

const DAY: i64 = 24 * 3600;
/// From here on a time is read as the clock time.
const FAR: i64 = 12 * 3600;

/// The zones read: UTC and the US abbreviations, as minutes east of UTC.
const ZONES: [(&str, i16); 11] = [
    ("UTC", 0),
    ("GMT", 0),
    ("Z", 0),
    ("EDT", -240),
    ("EST", -300),
    ("CDT", -300),
    ("CST", -360),
    ("MDT", -360),
    ("MST", -420),
    ("PDT", -420),
    ("PST", -480),
];

/// A clock time found at the start of some words.
#[derive(Debug, PartialEq, Eq)]
struct Clock {
    /// Seconds after midnight, in its zone.
    seconds: i64,
    /// Minutes east of UTC; `None` for `now`'s zone.
    zone: Option<i16>,
    /// Words it took.
    used: usize,
    /// Punctuation before and after it, kept.
    lead: String,
    trail: String,
}

/// `text`, its words single-spaced, with each clock time read relative to
/// `now` or as a clock time.
pub(crate) fn relative_times(text: &str, now: LocalTime) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut out: Vec<String> = Vec::with_capacity(words.len());
    let mut i = 0;
    while i < words.len() {
        let Some(clock) = clock_at(&words[i..]) else {
            out.push(words[i].to_string());
            i += 1;
            continue;
        };
        i += clock.used;
        let after_date = out.last().is_some_and(|w| is_date(w));
        let relative = (!after_date).then(|| relative(&clock, now)).flatten();
        let spoken = match relative {
            Some(mut spoken) => {
                // "at 16:13:48" is "three minutes ago".
                if clock.lead.is_empty()
                    && let Some(at) = out.pop_if(|w| *w == "at" || *w == "At")
                    && at == "At"
                {
                    spoken = capitalised(&spoken);
                }
                spoken
            }
            None => spoken_clock(&clock, now),
        };
        out.push(format!("{}{spoken}{}", clock.lead, clock.trail));
    }
    out.join(" ")
}

/// The clock time that `words` start with, if any.
fn clock_at(words: &[&str]) -> Option<Clock> {
    let (lead, core, trail) = split_punct(words[0]);
    let mut used = 1;
    let mut trail = trail.to_string();
    let mut zone = None;
    let mut meridiem = None;
    // Attached: `20:02:49Z`, `4:13pm`.
    let mut digits = core;
    if let Some(d) = core.strip_suffix('Z') {
        digits = d;
        zone = Some(0);
    } else if core.len() > 2
        // Split only on a character boundary: a word may end in a
        // character of more than one byte (`—`, `…`, `→`).
        && let Some((d, m)) = core.split_at_checked(core.len() - 2)
        && let Some(pm) = meridiem_of(m)
    {
        digits = d;
        meridiem = Some(pm);
    }
    let (hour, minute, second) = parse_digits(digits)?;
    // The words after it, while no punctuation ends it.
    let next = |used: usize| {
        words
            .get(used)
            .map(|w| split_punct(w))
            .filter(|(lead, _, _)| lead.is_empty())
    };
    if trail.is_empty()
        && meridiem.is_none()
        && zone.is_none()
        && let Some((_, word, t)) = next(used)
        && let Some(pm) = meridiem_of(word)
    {
        meridiem = Some(pm);
        trail = t.to_string();
        used += 1;
    }
    if trail.is_empty()
        && zone.is_none()
        && let Some((_, word, t)) = next(used)
        && let Some(offset) = zone_of(word)
    {
        zone = Some(offset);
        trail = t.to_string();
        used += 1;
    }
    let hour = match meridiem {
        Some(pm) => {
            if !(1..=12).contains(&hour) {
                return None;
            }
            hour % 12 + if pm { 12 } else { 0 }
        }
        None => hour,
    };
    if hour > 23 {
        return None;
    }
    Some(Clock {
        seconds: hour * 3600 + minute * 60 + second,
        zone,
        used,
        lead: lead.to_string(),
        trail,
    })
}

/// `H:MM` or `H:MM:SS`: one or two digits of hour, two of minute and
/// second.
fn parse_digits(text: &str) -> Option<(i64, i64, i64)> {
    let parts: Vec<&str> = text.split(':').collect();
    if !(2..=3).contains(&parts.len())
        || parts.iter().any(|p| !p.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    if !(1..=2).contains(&parts[0].len()) || parts[1..].iter().any(|p| p.len() != 2) {
        return None;
    }
    let n = |p: &str| p.parse::<i64>().ok();
    let hour = n(parts[0])?;
    let minute = n(parts[1])?;
    let second = parts.get(2).map_or(Some(0), |p| n(p))?;
    (minute < 60 && second < 60).then_some((hour, minute, second))
}

/// A word split into the punctuation before it, itself, and the
/// punctuation after it.
fn split_punct(word: &str) -> (&str, &str, &str) {
    let before = |c: char| matches!(c, '(' | '"' | '\'');
    let after = |c: char| matches!(c, ',' | ';' | '.' | '!' | '?' | ')' | '"' | '\'');
    let start = word.len() - word.trim_start_matches(before).len();
    let end = word.trim_end_matches(after).len().max(start);
    (&word[..start], &word[start..end], &word[end..])
}

/// `Some(true)` for PM, `Some(false)` for AM, in any case.
fn meridiem_of(word: &str) -> Option<bool> {
    match word.to_ascii_uppercase().as_str() {
        "AM" => Some(false),
        "PM" => Some(true),
        _ => None,
    }
}

fn zone_of(word: &str) -> Option<i16> {
    ZONES.iter().find(|(z, _)| *z == word).map(|(_, o)| *o)
}

/// `2026-10-23`, perhaps with punctuation after it.
fn is_date(word: &str) -> bool {
    let (_, core, _) = split_punct(word);
    let parts: Vec<&str> = core.split('-').collect();
    parts.len() == 3
        && [4, 2, 2]
            .iter()
            .zip(&parts)
            .all(|(n, p)| p.len() == *n && p.bytes().all(|b| b.is_ascii_digit()))
}

/// The time relative to `now`, or `None` when it is 12 hours or more away.
fn relative(clock: &Clock, now: LocalTime) -> Option<String> {
    let offset = i64::from(clock.zone.unwrap_or(now.utc_offset_minutes));
    let now_there =
        (now.seconds() - i64::from(now.utc_offset_minutes) * 60 + offset * 60).rem_euclid(DAY);
    let past = now_there - clock.seconds;
    let away = past.abs();
    if away >= FAR {
        return None;
    }
    if away < 10 {
        return Some("just now".to_string());
    }
    let span = if away < 60 {
        format!("{} seconds", number(away))
    } else if away < 3600 {
        match away / 60 {
            1 => "a minute".to_string(),
            m => format!("{} minutes", number(m)),
        }
    } else {
        match (away + 1800) / 3600 {
            1 => "about an hour".to_string(),
            h => format!("about {} hours", number(h)),
        }
    };
    Some(if past >= 0 {
        format!("{span} ago")
    } else {
        format!("in {span}")
    })
}

/// The clock time in `now`'s zone, as "four thirteen PM".
fn spoken_clock(clock: &Clock, now: LocalTime) -> String {
    let offset = i64::from(clock.zone.unwrap_or(now.utc_offset_minutes));
    let local =
        (clock.seconds - offset * 60 + i64::from(now.utc_offset_minutes) * 60).rem_euclid(DAY);
    let (hour, minute) = (local / 3600, local / 60 % 60);
    let half = if hour < 12 { "AM" } else { "PM" };
    let hour = match hour % 12 {
        0 => 12,
        h => h,
    };
    match minute {
        0 => format!("{} {half}", number(hour)),
        1..=9 => format!("{} oh {} {half}", number(hour), number(minute)),
        m => format!("{} {} {half}", number(hour), number(m)),
    }
}

/// 0 to 59 in words.
fn number(n: i64) -> String {
    const ONES: [&str; 20] = [
        "zero",
        "one",
        "two",
        "three",
        "four",
        "five",
        "six",
        "seven",
        "eight",
        "nine",
        "ten",
        "eleven",
        "twelve",
        "thirteen",
        "fourteen",
        "fifteen",
        "sixteen",
        "seventeen",
        "eighteen",
        "nineteen",
    ];
    const TENS: [&str; 6] = ["", "", "twenty", "thirty", "forty", "fifty"];
    match n {
        0..=19 => ONES[n as usize].to_string(),
        _ if n % 10 == 0 => TENS[(n / 10) as usize].to_string(),
        _ => format!("{} {}", TENS[(n / 10) as usize], ONES[(n % 10) as usize]),
    }
}

fn capitalised(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 16:16:48 EDT (20:16:48 UTC).
    const NOW: LocalTime = LocalTime {
        hour: 16,
        minute: 16,
        second: 48,
        utc_offset_minutes: -240,
    };

    fn read(text: &str) -> String {
        relative_times(text, NOW)
    }

    #[test]
    fn each_form_is_read_relative_to_now() {
        assert_eq!(read("Done at 16:13:48."), "Done three minutes ago.");
        assert_eq!(
            read("Built at 16:13, then pushed."),
            "Built three minutes ago, then pushed."
        );
        assert_eq!(
            read("At 16:13:48 the tests passed."),
            "Three minutes ago the tests passed."
        );
        assert_eq!(
            read("It ran at 20:02:49 UTC."),
            "It ran thirteen minutes ago."
        );
        assert_eq!(read("It ran at 20:02:49Z."), "It ran thirteen minutes ago.");
        assert_eq!(read("Merged at 4:13 PM."), "Merged three minutes ago.");
        assert_eq!(read("Merged at 4:13 PM EDT."), "Merged three minutes ago.");
        assert_eq!(read("Merged at 3:13 pm CDT"), "Merged three minutes ago");
        assert_eq!(read("Merged at 4:13pm."), "Merged three minutes ago.");
        assert_eq!(read("(16:13:48)"), "(three minutes ago)");
        assert_eq!(read("at 12:30 PM PST"), "in thirteen minutes");
    }

    #[test]
    fn each_span_has_its_words() {
        assert_eq!(read("at 16:16:40"), "just now");
        assert_eq!(read("at 16:16:08"), "forty seconds ago");
        assert_eq!(read("at 16:15:30"), "a minute ago");
        assert_eq!(read("at 15:46:48"), "thirty minutes ago");
        assert_eq!(read("at 15:10"), "about an hour ago");
        assert_eq!(read("at 13:00"), "about three hours ago");
        assert_eq!(read("at 4:30 AM"), "about twelve hours ago");
    }

    #[test]
    fn a_future_time_reads_ahead() {
        assert_eq!(read("Due at 16:20:00."), "Due in three minutes.");
        assert_eq!(read("Due at 16:17:30."), "Due in forty two seconds.");
        assert_eq!(read("Due at 16:18"), "Due in a minute");
        assert_eq!(read("Due at 19:00"), "Due in about three hours");
        assert_eq!(read("at 16:16:50"), "just now");
    }

    #[test]
    fn twelve_hours_or_more_is_the_clock_time() {
        assert_eq!(read("Started at 4:13:00."), "Started at four thirteen AM.");
        assert_eq!(read("Started at 4:16:48"), "Started at four sixteen AM");
        assert_eq!(read("Due at 4:05"), "Due at four oh five AM");
        assert_eq!(read("at 0:00"), "at twelve AM");
        // Converted to now's zone: 08:00 UTC is 4 AM EDT.
        assert_eq!(read("at 08:00 UTC"), "at four AM");
        // After a date: perhaps not today.
        assert_eq!(
            read("on 2026-10-22 16:13:48"),
            "on 2026-10-22 four thirteen PM"
        );
    }

    #[test]
    fn not_times_are_left_alone() {
        for text in [
            "3 s",
            "10 seconds",
            "2026-10-23",
            "ratio 1:2",
            "line 12:5",
            "port localhost:8080",
            "at 24:00",
            "at 16:60",
            "at 13:00 PM",
            "version 0.1.2",
        ] {
            assert_eq!(read(text), text);
        }
    }

    #[test]
    fn a_word_ending_in_a_wide_character_is_left_alone() {
        // The owner's report of 2026-10-30 15:12 EDT: a word ending in a
        // character of more than one byte (a dash, an ellipsis, an arrow,
        // a tick) was split inside that character, and the reading
        // panicked.
        for text in [
            "Built it \u{2014} all green.",
            "Working on it\u{2026}",
            "old \u{2192} new",
            "Tests pass \u{2713}",
            "\u{2014}",
            "at 16:13\u{2014}then pushed",
        ] {
            assert_eq!(read(text), text);
        }
        assert_eq!(
            read("Done at 16:13:48 \u{2014} pushed."),
            "Done three minutes ago \u{2014} pushed."
        );
    }
}
