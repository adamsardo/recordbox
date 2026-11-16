//! Intelligent (smart) playlists: a list of conditions matched with "all" or "any".

use crate::model::Track;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Field {
    Title,
    Artist,
    Album,
    AlbumArtist,
    Genre,
    Label,
    Remixer,
    OriginalArtist,
    Composer,
    Comments,
    MixName,
    Grouping,
    FileName,
    Key,
    Bpm,
    Rating,
    Color,
    Year,
    Time,
    Bitrate,
    SampleRate,
    PlayCount,
    DateAdded,
    ReleaseDate,
    MyTag,
    FileType,
}

pub const FIELDS: [Field; 26] = [
    Field::Title,
    Field::Artist,
    Field::Album,
    Field::AlbumArtist,
    Field::Genre,
    Field::Label,
    Field::Remixer,
    Field::OriginalArtist,
    Field::Composer,
    Field::Comments,
    Field::MixName,
    Field::Grouping,
    Field::FileName,
    Field::Key,
    Field::Bpm,
    Field::Rating,
    Field::Color,
    Field::Year,
    Field::Time,
    Field::Bitrate,
    Field::SampleRate,
    Field::PlayCount,
    Field::DateAdded,
    Field::ReleaseDate,
    Field::MyTag,
    Field::FileType,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldType {
    Text,
    Number,
    Date,
    Key,
    Rating,
    Color,
    Tag,
}

impl Field {
    pub fn label(self) -> &'static str {
        match self {
            Field::Title => "Track Title",
            Field::Artist => "Artist",
            Field::Album => "Album",
            Field::AlbumArtist => "Album artist",
            Field::Genre => "Genre",
            Field::Label => "Label",
            Field::Remixer => "Remixer",
            Field::OriginalArtist => "Original artist",
            Field::Composer => "Composer",
            Field::Comments => "Comments",
            Field::MixName => "Mix Name",
            Field::Grouping => "Grouping",
            Field::FileName => "File name",
            Field::Key => "Key",
            Field::Bpm => "BPM",
            Field::Rating => "Rating",
            Field::Color => "Color",
            Field::Year => "Year",
            Field::Time => "Time",
            Field::Bitrate => "Bitrate",
            Field::SampleRate => "Sample Rate",
            Field::PlayCount => "DJ play count",
            Field::DateAdded => "Date Added",
            Field::ReleaseDate => "Release Date",
            Field::MyTag => "My Tag",
            Field::FileType => "File Type",
        }
    }
    pub fn ftype(self) -> FieldType {
        match self {
            Field::Key => FieldType::Key,
            Field::Bpm | Field::Year | Field::Time | Field::Bitrate | Field::SampleRate | Field::PlayCount => FieldType::Number,
            Field::Rating => FieldType::Rating,
            Field::Color => FieldType::Color,
            Field::DateAdded | Field::ReleaseDate => FieldType::Date,
            Field::MyTag => FieldType::Tag,
            _ => FieldType::Text,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Op {
    Is,
    IsNot,
    Contains,
    NotContains,
    StartsWith,
    EndsWith,
    Greater,
    Less,
    InRange,
    InLast,
    NotInLast,
}

impl Op {
    pub fn label(self) -> &'static str {
        match self {
            Op::Is => "is",
            Op::IsNot => "is not",
            Op::Contains => "contains",
            Op::NotContains => "does not contain",
            Op::StartsWith => "starts with",
            Op::EndsWith => "ends with",
            Op::Greater => "is greater than",
            Op::Less => "is less than",
            Op::InRange => "is in the range",
            Op::InLast => "is in the last",
            Op::NotInLast => "is not in the last",
        }
    }
    pub fn for_type(t: FieldType) -> &'static [Op] {
        match t {
            FieldType::Text => &[Op::Is, Op::IsNot, Op::Contains, Op::NotContains, Op::StartsWith, Op::EndsWith],
            FieldType::Number | FieldType::Rating => &[Op::Is, Op::IsNot, Op::Greater, Op::Less, Op::InRange],
            FieldType::Date => &[Op::Is, Op::IsNot, Op::Greater, Op::Less, Op::InRange, Op::InLast, Op::NotInLast],
            FieldType::Key | FieldType::Color => &[Op::Is, Op::IsNot],
            FieldType::Tag => &[Op::Contains, Op::NotContains],
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Condition {
    pub field: Field,
    pub op: Op,
    pub value: String,
    pub value2: String,
    /// For "in the last": true = months, false = days.
    #[serde(default)]
    pub months: bool,
}

impl Default for Condition {
    fn default() -> Self {
        Condition { field: Field::Genre, op: Op::Contains, value: String::new(), value2: String::new(), months: false }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SmartRule {
    pub match_all: bool,
    pub conditions: Vec<Condition>,
    #[serde(default = "yes")]
    pub auto_update: bool,
}
fn yes() -> bool {
    true
}

impl Default for SmartRule {
    fn default() -> Self {
        SmartRule { match_all: true, conditions: vec![Condition::default()], auto_update: true }
    }
}

fn text_of(t: &Track, f: Field) -> String {
    match f {
        Field::Title => t.display_title(),
        Field::Artist => t.artist.clone(),
        Field::Album => t.album.clone(),
        Field::AlbumArtist => t.album_artist.clone(),
        Field::Genre => t.genre.clone(),
        Field::Label => t.label.clone(),
        Field::Remixer => t.remixer.clone(),
        Field::OriginalArtist => t.original_artist.clone(),
        Field::Composer => t.composer.clone(),
        Field::Comments => t.comment.clone(),
        Field::MixName => t.mix_name.clone(),
        Field::Grouping => t.grouping.clone(),
        Field::FileName => t.file_name(),
        Field::FileType => t.file_type.to_ascii_uppercase(),
        _ => String::new(),
    }
}

fn num_of(t: &Track, f: Field) -> f64 {
    match f {
        Field::Bpm => t.bpm,
        Field::Year => t.year as f64,
        Field::Time => t.duration,
        Field::Bitrate => t.bitrate as f64,
        Field::SampleRate => t.sample_rate as f64,
        Field::PlayCount => t.play_count as f64,
        Field::Rating => t.rating as f64,
        _ => 0.0,
    }
}

fn parse_num(f: Field, s: &str) -> f64 {
    let s = s.trim();
    if f == Field::Time {
        // mm:ss or seconds
        if let Some((m, sec)) = s.split_once(':') {
            return m.trim().parse::<f64>().unwrap_or(0.0) * 60.0 + sec.trim().parse::<f64>().unwrap_or(0.0);
        }
    }
    s.parse::<f64>().unwrap_or(0.0)
}

fn date_days(s: &str) -> Option<i64> {
    let d = chrono::NaiveDate::parse_from_str(s.get(..10).unwrap_or(s), "%Y-%m-%d").ok()?;
    Some(d.num_days_from_ce() as i64)
}
use chrono::Datelike;

/// Collation used for text comparison: case-insensitive, accents folded.
pub fn fold(s: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    s.nfkd().filter(|c| !unicode_normalization::char::is_combining_mark(*c)).collect::<String>().to_lowercase()
}

pub fn matches(rule: &SmartRule, t: &Track, tags: &HashSet<i64>, today: i64) -> bool {
    if rule.conditions.is_empty() {
        return false;
    }
    let mut results = rule.conditions.iter().map(|c| cond_matches(c, t, tags, today));
    if rule.match_all {
        results.all(|x| x)
    } else {
        results.any(|x| x)
    }
}

fn cond_matches(c: &Condition, t: &Track, tags: &HashSet<i64>, today: i64) -> bool {
    match c.field.ftype() {
        FieldType::Text => {
            let v = fold(&text_of(t, c.field));
            let q = fold(c.value.trim());
            match c.op {
                Op::Is => v == q,
                Op::IsNot => v != q,
                Op::Contains => v.contains(&q),
                Op::NotContains => !v.contains(&q),
                Op::StartsWith => v.starts_with(&q),
                Op::EndsWith => v.ends_with(&q),
                _ => false,
            }
        }
        FieldType::Number | FieldType::Rating => {
            let v = num_of(t, c.field);
            let a = parse_num(c.field, &c.value);
            let b = parse_num(c.field, &c.value2);
            let eq = |x: f64, y: f64| if c.field == Field::Bpm { (x - y).abs() < 0.005 } else { (x - y).abs() < 0.5 };
            match c.op {
                Op::Is => eq(v, a),
                Op::IsNot => !eq(v, a),
                Op::Greater => v > a,
                Op::Less => v < a,
                Op::InRange => v >= a.min(b) && v <= a.max(b),
                _ => false,
            }
        }
        FieldType::Date => {
            let s = if c.field == Field::DateAdded { &t.date_added } else { &t.release_date };
            let Some(v) = date_days(s) else { return matches!(c.op, Op::IsNot | Op::NotInLast) };
            let a = date_days(&c.value);
            let b = date_days(&c.value2);
            match c.op {
                Op::Is => a == Some(v),
                Op::IsNot => a != Some(v),
                Op::Greater => a.map(|a| v > a).unwrap_or(false),
                Op::Less => a.map(|a| v < a).unwrap_or(false),
                Op::InRange => match (a, b) {
                    (Some(a), Some(b)) => v >= a.min(b) && v <= a.max(b),
                    _ => false,
                },
                Op::InLast | Op::NotInLast => {
                    let n = c.value.trim().parse::<i64>().unwrap_or(0);
                    let span = if c.months { n * 30 } else { n };
                    let inside = today - v <= span && v <= today;
                    if c.op == Op::InLast {
                        inside
                    } else {
                        !inside
                    }
                }
                _ => false,
            }
        }
        FieldType::Key => {
            let k = crate::model::parse_key(&c.value);
            match c.op {
                Op::Is => t.key == k && k >= 0,
                Op::IsNot => t.key != k,
                _ => false,
            }
        }
        FieldType::Color => {
            let k = c.value.trim().parse::<u8>().unwrap_or(0);
            match c.op {
                Op::Is => t.color == k,
                Op::IsNot => t.color != k,
                _ => false,
            }
        }
        FieldType::Tag => {
            let id = c.value.trim().parse::<i64>().unwrap_or(-1);
            match c.op {
                Op::Contains => tags.contains(&id),
                Op::NotContains => !tags.contains(&id),
                _ => false,
            }
        }
    }
}

pub fn today_days() -> i64 {
    chrono::Local::now().date_naive().num_days_from_ce() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(field: Field, op: Op, value: &str, value2: &str) -> Condition {
        Condition { field, op, value: value.into(), value2: value2.into(), months: false }
    }
    fn day(s: &str) -> i64 {
        date_days(s).unwrap()
    }

    #[test]
    fn evaluates_each_field_type() {
        let t = Track { title: "Café del Mar".into(), artist: "Energy 52".into(), genre: "Trance".into(), bpm: 128.0, rating: 4, color: 3, key: 7, date_added: "2026-11-10".into(), duration: 300.0, ..Default::default() };
        let tags: HashSet<i64> = [42].into_iter().collect();
        let today = day("2026-11-17");
        let one = |cond: Condition| matches(&SmartRule { match_all: true, conditions: vec![cond], auto_update: true }, &t, &tags, today);
        // Text: case- and accent-insensitive.
        assert!(one(c(Field::Title, Op::Contains, "cafe", "")));
        assert!(one(c(Field::Title, Op::StartsWith, "CAFÉ", "")));
        assert!(one(c(Field::Genre, Op::Is, "trance", "")));
        assert!(!one(c(Field::Genre, Op::IsNot, "Trance", "")));
        assert!(one(c(Field::Artist, Op::NotContains, "techno", "")));
        // Numbers: ranges are inclusive and either order; BPM compares to 0.01.
        assert!(one(c(Field::Bpm, Op::InRange, "130", "126")));
        assert!(one(c(Field::Bpm, Op::Is, "128.00", "")));
        assert!(!one(c(Field::Bpm, Op::Is, "128.02", "")));
        assert!(one(c(Field::Rating, Op::Greater, "3", "")));
        assert!(one(c(Field::Time, Op::Less, "5:01", "")));
        // Dates: in the last N days / months.
        assert!(one(c(Field::DateAdded, Op::InLast, "7", "")));
        assert!(!one(c(Field::DateAdded, Op::InLast, "6", "")));
        assert!(one(c(Field::DateAdded, Op::NotInLast, "3", "")));
        assert!(one(Condition { months: true, ..c(Field::DateAdded, Op::InLast, "1", "") }));
        // Key, colour, My Tag.
        assert!(one(c(Field::Key, Op::Is, &crate::model::key_name(7, false), "")));
        assert!(one(c(Field::Color, Op::Is, "3", "")));
        assert!(one(c(Field::MyTag, Op::Contains, "42", "")));
        assert!(one(c(Field::MyTag, Op::NotContains, "41", "")));
        // A track without a date never matches "in the last", always "not in the last".
        let undated = Track { date_added: String::new(), ..t.clone() };
        let r = |op| SmartRule { match_all: true, conditions: vec![c(Field::DateAdded, op, "7", "")], auto_update: true };
        assert!(!matches(&r(Op::InLast), &undated, &tags, today));
        assert!(matches(&r(Op::NotInLast), &undated, &tags, today));
    }

    #[test]
    fn all_and_any() {
        let t = Track { genre: "House".into(), bpm: 124.0, ..Default::default() };
        let tags = HashSet::new();
        let conds = vec![c(Field::Genre, Op::Is, "house", ""), c(Field::Bpm, Op::Greater, "130", "")];
        assert!(!matches(&SmartRule { match_all: true, conditions: conds.clone(), auto_update: true }, &t, &tags, 0));
        assert!(matches(&SmartRule { match_all: false, conditions: conds, auto_update: true }, &t, &tags, 0));
        // No conditions match nothing.
        assert!(!matches(&SmartRule { match_all: true, conditions: vec![], auto_update: true }, &t, &tags, 0));
    }
}
