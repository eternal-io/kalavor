#![allow(non_snake_case)]

use super::*;

/// Parser macro used to combine predicates, produces a closure that accepts any character except these specified.
#[macro_export]
macro_rules! not {
    ( $($preds:expr),+ $(,)? ) => {
        |ch: char| not!( @ ch $($preds),+ )
    };

    ( @ $ch:ident $pred:expr, $($preds:expr),* ) => {
        !$pred($ch) || not!( @ $ch $($preds),* )
    };

    ( @ $ch:ident $pred:expr ) => {
        !$pred($ch)
    };
}

/// Parser macro used to combine predicates, produces a closure that only accepts these specified characters.
#[macro_export]
macro_rules! any {
    ( $($preds:expr),+ $(,)? ) => {
        |ch: char| any!( @ ch $($preds),+ )
    };

    ( @ $ch:ident $pred:expr, $($preds:expr),* ) => {
        $pred($ch) || any!( @ $ch $($preds),* )
    };

    ( @ $ch:ident $pred:expr ) => {
        $pred($ch)
    };
}

//==================================================================================================

pub const fn OneOf(chars: &'static str) -> impl 'static + FnMut(char) -> bool {
    move |ch| chars.contains(ch)
}

pub const fn NoneOf(chars: &'static str) -> impl 'static + FnMut(char) -> bool {
    move |ch| !chars.contains(ch)
}

pub const fn RangeOf<R>(range: R) -> impl 'static + FnMut(char) -> bool
where
    R: 'static + RangeBounds<char>,
{
    move |ch| range.contains(&ch)
}

//==================================================================================================

pub const fn Any(ch: char) -> bool {
    let _ = ch;
    true
}

pub const fn Newline(ch: char) -> bool {
    ch == '\n'
}

pub const fn Whitespace(ch: char) -> bool {
    matches!(ch, '\n' | '\t' | '\r' | '\x0b' | '\x0c' | '\x20')
}

#[cfg(feature = "parser-unicode")]
pub mod unc {
    pub fn XidStart(ch: char) -> bool {
        unicode_ident::is_xid_start(ch)
    }

    pub fn XidContinue(ch: char) -> bool {
        unicode_ident::is_xid_continue(ch)
    }
}
