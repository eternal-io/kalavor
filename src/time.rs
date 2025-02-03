//! The flavored datetime format, e.g. *`A01123-0456-0789`*.
//!
//! This module has fully re-exported the [time] crate, provided convenience functions as listed below.

extern crate alloc;

use alloc::{boxed::Box, vec::Vec};

pub use time::*;

#[cfg(feature = "std")]
pub fn now() -> Box<str> {
    of(OffsetDateTime::now_utc())
}
#[cfg(feature = "std")]
pub fn now_precise() -> Box<str> {
    precise_of(OffsetDateTime::now_utc())
}

pub fn of(dt: OffsetDateTime) -> Box<str> {
    format::<false>(dt)
}
pub fn precise_of(dt: OffsetDateTime) -> Box<str> {
    format::<true>(dt)
}

#[allow(clippy::uninit_vec)]
fn format<const MICRO: bool>(dt: OffsetDateTime) -> Box<str> {
    let dt = dt.to_offset(UtcOffset::from_whole_seconds(8 * 3600).unwrap());
    let len = 16 + if MICRO { 5 } else { 0 };
    let mut out = Vec::with_capacity(len);
    unsafe { out.set_len(len) }

    /* A01123-0456-0789.0137 */

    let cap = dt.year().is_positive();
    let yy = (dt.year() - 2022).rem_euclid(200) as u8;

    let month = dt.month() as u8;
    let day = dt.day();

    let hour = dt.hour();
    let minute = dt.minute();

    let second = dt.second();
    let milli = (dt.millisecond() / 10) as u8;

    out[0] = if cap { b'A' } else { b'a' } + yy / 10;
    out[1] = b'0' + yy % 10;
    out[2] = b'0' + month / 10;
    out[3] = b'0' + month % 10;
    out[4] = b'0' + day / 10;
    out[5] = b'0' + day % 10;

    out[6] = b'-';

    out[7] = b'0' + hour / 10;
    out[8] = b'0' + hour % 10;
    out[9] = b'0' + minute / 10;
    out[10] = b'0' + minute % 10;

    out[11] = b'-';

    out[12] = b'0' + second / 10;
    out[13] = b'0' + second % 10;
    out[14] = b'0' + milli / 10;
    out[15] = b'0' + milli % 10;

    if MICRO {
        let micro = dt.microsecond() % 10000;
        let high = (micro / 100) as u8;
        let low = (micro % 100) as u8;

        out[16] = b'.';

        out[17] = b'0' + high / 10;
        out[18] = b'0' + high % 10;
        out[19] = b'0' + low / 10;
        out[20] = b'0' + low % 10;
    }

    unsafe { alloc::str::from_boxed_utf8_unchecked(out.into_boxed_slice()) }
}

#[test]
fn test() {
    dbg!(now());
    dbg!(now_precise());

    assert_eq!(
        precise_of(OffsetDateTime::new_in_offset(
            Date::from_calendar_date(2025, Month::January, 28).unwrap(),
            Time::from_hms_micro(23, 04, 0, 1123).unwrap(),
            UtcOffset::from_whole_seconds(8 * 3600).unwrap(),
        ))
        .as_ref(),
        "A30128-2304-0000.1123"
    );
}
