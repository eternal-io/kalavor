use std::ops::{Range, RangeFrom, RangeFull, RangeInclusive, RangeTo, RangeToInclusive};

pub trait URangeBounds {
    fn contains(&self, times: usize) -> bool;
    fn want_more(&self, times: usize) -> bool;
}

#[rustfmt::skip]
impl URangeBounds for usize {
    fn contains(&self, times: usize) -> bool { times == *self }
    fn want_more(&self, times: usize) -> bool { times < *self }
}
#[rustfmt::skip]
impl URangeBounds for RangeFull {
    fn contains(&self, _t: usize) -> bool { true }
    fn want_more(&self, _t: usize) -> bool { true }
}
#[rustfmt::skip]
impl URangeBounds for RangeFrom<usize> {
    fn contains(&self, times: usize) -> bool { self.contains(&times) }
    fn want_more(&self, _t: usize) -> bool { true }
}
#[rustfmt::skip]
impl URangeBounds for Range<usize> {
    fn contains(&self, times: usize) -> bool { self.contains(&times) }
    fn want_more(&self, times: usize) -> bool { times + 1 < self.end }
}
#[rustfmt::skip]
impl URangeBounds for RangeTo<usize> {
    fn contains(&self, times: usize) -> bool { self.contains(&times) }
    fn want_more(&self, times: usize) -> bool { times + 1 < self.end }
}
#[rustfmt::skip]
impl URangeBounds for RangeInclusive<usize> {
    fn contains(&self, times: usize) -> bool { self.contains(&times) }
    fn want_more(&self, times: usize) -> bool { times < *self.end() }
}
#[rustfmt::skip]
impl URangeBounds for RangeToInclusive<usize> {
    fn contains(&self, times: usize) -> bool { self.contains(&times) }
    fn want_more(&self, times: usize) -> bool { times < self.end }
}

//------------------------------------------------------------------------------

/// Trait that predicates a set of characters.
pub trait Predicate {
    fn predicate(&self, ch: char) -> bool;
}

impl<T: 'static + Fn(char) -> bool> Predicate for T {
    fn predicate(&self, ch: char) -> bool {
        self(ch)
    }
}

impl Predicate for char {
    fn predicate(&self, ch: char) -> bool {
        ch == *self
    }
}
impl Predicate for &str {
    fn predicate(&self, ch: char) -> bool {
        self.contains(ch)
    }
}
impl<const N: usize> Predicate for [char; N] {
    fn predicate(&self, ch: char) -> bool {
        self.contains(&ch)
    }
}
impl Predicate for Range<char> {
    fn predicate(&self, ch: char) -> bool {
        self.contains(&ch)
    }
}
impl Predicate for RangeInclusive<char> {
    fn predicate(&self, ch: char) -> bool {
        self.contains(&ch)
    }
}

//------------------------------------------------------------------------------

#[inline(always)]
const fn utf8_first_byte(ch: char) -> u8 {
    let code = ch as u32;
    match ch.len_utf8() {
        1 => ch as u8,
        2 => (code >> 6 & 0x1F) as u8 | 0b1100_0000,
        3 => (code >> 12 & 0x0F) as u8 | 0b1110_0000,
        4 => (code >> 18 & 0x07) as u8 | 0b1111_0000,
        _ => unreachable!(),
    }
}

/// Trait that matches a set of strings.
pub trait Pattern {
    type Discriminant;

    /// Returns the max length of the possible sub-patterns.
    fn indicate(&self, begin: u8) -> Option<usize>;

    /// Returns the length of the matched sub-pattern, and the index of the sub-pattern.
    fn matches(&self, content: &str) -> Option<(usize, Self::Discriminant)>;
}

impl Pattern for char {
    type Discriminant = Self;

    fn indicate(&self, begin: u8) -> Option<usize> {
        (utf8_first_byte(*self) == begin).then_some(self.len_utf8())
    }

    fn matches(&self, content: &str) -> Option<(usize, Self::Discriminant)> {
        content.starts_with(*self).then_some((self.len_utf8(), *self))
    }
}

impl Pattern for &str {
    type Discriminant = Self;

    fn indicate(&self, begin: u8) -> Option<usize> {
        self.as_bytes()
            .first()
            .and_then(|&b| (b == begin).then_some(self.len()))
    }

    fn matches(&self, content: &str) -> Option<(usize, Self::Discriminant)> {
        content.starts_with(self).then_some((self.len(), self))
    }
}

impl<const N: usize> Pattern for [char; N] {
    type Discriminant = char;

    fn indicate(&self, begin: u8) -> Option<usize> {
        self.iter()
            .filter_map(|&ch| (utf8_first_byte(ch) == begin).then_some(ch.len_utf8()))
            .max()
    }

    fn matches(&self, content: &str) -> Option<(usize, Self::Discriminant)> {
        self.iter()
            .find(|&&ch| content.starts_with(ch))
            .map(|&i| (i.len_utf8(), i))
    }
}

impl<'a, const N: usize> Pattern for [&'a str; N] {
    type Discriminant = &'a str;

    fn indicate(&self, begin: u8) -> Option<usize> {
        self.iter()
            .filter_map(|s| s.as_bytes().first().and_then(|&b| (b == begin).then_some(s.len())))
            .max()
    }

    fn matches(&self, content: &str) -> Option<(usize, Self::Discriminant)> {
        self.iter().find(|&&s| content.starts_with(s)).map(|&i| (i.len(), i))
    }
}

//==================================================================================================

/// *(parser)* Combine predicates, produce a new predicate that accepts only these specified characters.
#[macro_export]
macro_rules! all {
    ( $($preds:expr),+ $(,)? ) => {
        move |ch: char| all!( @ ch $($preds),+ )
    };

    ( @ $ch:ident $pred:expr, $($preds:expr),* ) => {
        $pred.predicate($ch) || all!( @ $ch $($preds),* )
    };

    ( @ $ch:ident $pred:expr ) => {
        $pred.predicate($ch)
    };
}

/// *(parser)* Combine predicates, produce a new predicate that accepts any character except these specified.
#[macro_export]
macro_rules! not {
    ( $($preds:expr),+ $(,)? ) => {
        move |ch: char| not!( @ ch $($preds),+ )
    };

    ( @ $ch:ident $pred:expr, $($preds:expr),* ) => {
        !$pred.predicate($ch) && not!( @ $ch $($preds),* )
    };

    ( @ $ch:ident $pred:expr ) => {
        !$pred.predicate($ch)
    };
}

/// *(parser)* Implement [`Pattern`] for enumerations of a set of tokens conveniently.
#[macro_export]
macro_rules! token_sets {
    ( $(
        $(#[$attr:meta])*
        $vis:vis enum $name:ident {
            $($key:ident = $word:literal),+
            $(,)?
        }
    )* ) => { $(
        $(#[$attr])*
        $vis enum $name { $(
            #[doc = concat!("Token ``````` ", $word, " ```````")]
            $key,
        )+ }

        impl $name {
            pub const fn len(&self) -> usize {
                self.text().len()
            }

            pub const fn text(&self) -> &'static str {
                match self { $(
                    Self::$key => $word,
                )+ }
            }
        }

        impl Pattern for $name {
            type Discriminant = Self;

            #[allow(unused_variables, unused_mut)]
            fn indicate(&self, begin: u8) -> Option<usize> {
                let mut max_len = 0usize;
            $(
                if $word.as_bytes()[0] == begin && $word.len() > max_len {
                    max_len = $word.len();
                }
            )+
                (max_len != 0).then_some(max_len)
            }

            #[allow(unused_variables)]
            fn matches(&self, content: &str) -> Option<(usize, Self::Discriminant)> {
            $(
                if content.starts_with($word) {
                    return Some(($word.len(), Self::$key))
                }
            )+
                None
            }
        }
    )* };
}

//==================================================================================================

/// Any character.
pub const fn any(ch: char) -> bool {
    let _ = ch;
    true
}

/// ASCII newline.
pub const fn newline(ch: char) -> bool {
    ch == '\n'
}

/// ASCII whitespace.
///
/// Note that this is different from [`char::is_ascii_whitespace`].
/// This includes U+000B VERTICAL TAB.
pub const fn whitespace(ch: char) -> bool {
    matches!(ch, '\n' | '\t' | '\r' | '\x0b' | '\x0c' | '\x20')
}

/// Any ASCII character.
pub const fn ascii(ch: char) -> bool {
    ch.is_ascii()
}
/// ASCII alphabetic.
pub const fn alphabetic(ch: char) -> bool {
    ch.is_ascii_alphabetic()
}
/// ASCII alphanumeric.
pub const fn alphanumeric(ch: char) -> bool {
    ch.is_ascii_alphanumeric()
}

/// ASCII decimal digit.
pub const fn digit(ch: char) -> bool {
    ch.is_ascii_digit()
}
/// ASCII hexadecimal digit.
pub const fn hexdigit(ch: char) -> bool {
    ch.is_ascii_hexdigit()
}
/// ASCII octal digit.
pub const fn octdigit(ch: char) -> bool {
    matches!(ch, '0'..='7')
}
/// ASCII binary digit.
pub const fn bindigit(ch: char) -> bool {
    matches!(ch, '0' | '1')
}

/// Unicode XID_START character.
#[cfg(feature = "parser-xid")]
pub fn xid_start(ch: char) -> bool {
    unicode_ident::is_xid_start(ch)
}

/// Unicode XID_CONTINUE character.
#[cfg(feature = "parser-xid")]
pub fn xid_continue(ch: char) -> bool {
    unicode_ident::is_xid_continue(ch)
}
