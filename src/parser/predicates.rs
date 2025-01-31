use std::ops::{Range, RangeInclusive};

pub trait Predicate {
    fn predicate(&self, ch: char) -> bool;
}

impl<T: 'static + Fn(char) -> bool> Predicate for T {
    fn predicate(&self, ch: char) -> bool {
        self(ch)
    }
}

impl Predicate for &str {
    fn predicate(&self, ch: char) -> bool {
        self.contains(ch)
    }
}
impl Predicate for &[char] {
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

//==================================================================================================

/// Parser macro used to combine predicates, produces a closure that accepts any character except these specified.
#[macro_export]
macro_rules! not {
    ( $($preds:expr),+ $(,)? ) => {
        move |ch: char| not!( @ ch $($preds),+ )
    };

    ( @ $ch:ident $pred:expr, $($preds:expr),* ) => {
        !$pred.predicate($ch) || not!( @ $ch $($preds),* )
    };

    ( @ $ch:ident $pred:expr ) => {
        !$pred.predicate($ch)
    };
}

/// Parser macro used to combine predicates, produces a closure that only accepts these specified characters.
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

//==================================================================================================

pub const fn any(ch: char) -> bool {
    let _ = ch;
    true
}

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

/// `[\x00-\x7f]` ASCII character.
pub const fn ascii(ch: char) -> bool {
    ch.is_ascii()
}
/// `[A-Za-z]` ASCII alphabetic.
pub const fn alphabetic(ch: char) -> bool {
    ch.is_ascii_alphabetic()
}
/// `[A-Za-z0-9]` ASCII alphanumeric.
pub const fn alphanumeric(ch: char) -> bool {
    ch.is_ascii_alphanumeric()
}

/// `[0-9]` ASCII decimal digit.
pub const fn digit(ch: char) -> bool {
    ch.is_ascii_digit()
}
/// `[0-9A-Fa-f]` ASCII hexadecimal digit.
pub const fn hex_digit(ch: char) -> bool {
    ch.is_ascii_hexdigit()
}
/// `[0-7]` ASCII octal digit.
pub const fn oct_digit(ch: char) -> bool {
    matches!(ch, '0'..='7')
}
/// `[0-1]` ASCII binary digit.
pub const fn bin_digit(ch: char) -> bool {
    matches!(ch, '0' | '1')
}

/// Unicode XID_START.
#[cfg(feature = "parser-unicode")]
pub fn xid_start(ch: char) -> bool {
    unicode_ident::is_xid_start(ch)
}

/// Unicode XID_CONTINUE.
#[cfg(feature = "parser-unicode")]
pub fn xid_continue(ch: char) -> bool {
    unicode_ident::is_xid_continue(ch)
}
