use simdutf8::compat::from_utf8;
use std::{
    io::{Error, ErrorKind, Read, Result},
    ops::{Bound, RangeBounds},
    ptr,
    result::Result as StdResult,
};

#[macro_use]
pub mod predicates;

pub use predicates::*;

pub struct Utf8Reader<R: Read> {
    src: R,
    buf: Vec<u8>,
    buf_cap: usize,
    tot_read: usize,
    tot_consumed: usize,
    off_consumed: usize,
    off_valid: usize,
    off_raw: usize,
    peeked: Option<u8>,
    eof: bool,
}

impl<R: Read> Utf8Reader<R> {
    pub const INIT_CAP: usize = 16 * 1024;
    pub const GROW_CAP: usize = 16 * 1024;
    const THRES_SHRINK: usize = 2 * 1024;
    const THRES_EXTEND: usize = 4 * 1024;

    #[allow(clippy::uninit_vec)]
    pub fn new(src: R) -> Self {
        Self {
            src,
            buf: unsafe {
                let mut buf = Vec::with_capacity(Self::INIT_CAP);
                buf.set_len(Self::INIT_CAP);
                buf
            },
            buf_cap: Self::INIT_CAP,
            tot_read: 0,
            tot_consumed: 0,
            off_consumed: 0,
            off_valid: 0,
            off_raw: 0,
            peeked: None,
            eof: false,
        }
    }

    /// Returns the string of unconsumed, valid UTF-8 bytes.
    #[inline]
    pub fn content(&self) -> &str {
        unsafe { core::str::from_utf8_unchecked(self.buf.get_unchecked(self.off_consumed..self.off_valid)) }
    }

    /// Marks the leading `n` bytes of the content as consumed, they will disappear in the future content.
    ///
    /// # Panics
    ///
    /// Panics if the `n`th byte is not at a UTF-8 character boundary.
    pub fn consume(&mut self, n: usize) {
        if !self.content().is_char_boundary(n) {
            panic!("{} is not at a UTF-8 character boundary", n)
        }

        self.off_consumed += n;
    }

    /// Returns the count of totally consumed bytes.
    pub fn consumed(&self) -> usize {
        self.tot_consumed + self.off_consumed
    }

    /// Returns `true` if encountered the EOF and all bytes are consumed.
    pub fn exhausted(&self) -> bool {
        self.eof && self.off_consumed == self.off_valid
    }

    //------------------------------------------------------------------------------

    /// Pulls no more than [`INIT_CAP`](Self::INIT_CAP) bytes.
    ///
    /*  NOTE: The content may NOT be pinned. */
    pub fn pull(&mut self) -> Result<()> {
        if self.off_raw - self.off_consumed > Self::INIT_CAP {
            return Ok(());
        } else if self.off_raw + Self::THRES_SHRINK > Self::INIT_CAP {
            unsafe {
                ptr::copy(
                    self.buf.as_ptr().add(self.off_consumed),
                    self.buf.as_ptr() as *mut _,
                    self.off_raw - self.off_consumed,
                )
            }

            self.tot_consumed += self.off_consumed;

            self.off_raw -= self.off_consumed;
            self.off_valid -= self.off_consumed;
            self.off_consumed = 0;
        }

        self.buf_cap = Self::INIT_CAP;
        unsafe { self.buf.set_len(self.buf_cap) }

        if self.off_raw >= Self::INIT_CAP {
            return Ok(());
        }

        self.fetch(Self::pull)
    }

    /// Pulls more bytes, allows the content to grow infinitely (at most [`GROW_CAP`](Self::GROW_CAP) bytes each call).
    ///
    /*  NOTE: The content would be pinned. */
    pub fn pull_more(&mut self) -> Result<()> {
        if self.buf_cap - self.off_raw < Self::THRES_EXTEND {
            self.buf.reserve(Self::GROW_CAP);
            self.buf_cap += Self::GROW_CAP;
            unsafe { self.buf.set_len(self.buf_cap) }
        }

        self.fetch(Self::pull_more)
    }

    /// Pulls more bytes, makes the content has at least `n` bytes.
    ///
    /// Returns `Ok(false)` if encountered the EOF, indicates that unable to read such more bytes.
    ///
    /*  NOTE: The content would be pinned.  */
    pub fn pull_at_least(&mut self, n: usize) -> Result<bool> {
        loop {
            match self.content().len() < n {
                false => return Ok(true),
                true => match !self.eof {
                    false => return Ok(false),
                    true => self.pull_more()?,
                },
            }
        }
    }

    fn fetch(&mut self, rerun: fn(&mut Self) -> Result<()>) -> Result<()> {
        let len = unsafe { self.src.read(self.buf.get_unchecked_mut(self.off_raw..self.buf_cap))? };

        self.eof = len == 0;

        if !self.eof {
            self.off_raw += len;
            self.tot_read += len;

            match self.validate()? > 0 {
                true => Ok(()),
                false => rerun(self),
            }
        } else {
            match self.off_valid == self.off_raw {
                true => Ok(()),
                false => Err(Error::new(
                    ErrorKind::UnexpectedEof,
                    "incomplete UTF-8 code point at the end",
                )),
            }
        }
    }

    fn validate(&mut self) -> Result<usize> {
        let valid_len = match from_utf8(unsafe { self.buf.get_unchecked(self.off_valid..self.off_raw) }) {
            Ok(s) => s.len(),
            Err(e) => match e.error_len() {
                None => e.valid_up_to(),
                Some(n) => Err(Error::new(
                    ErrorKind::InvalidData,
                    format!("invalid UTF-8 bytes at slice {}+{}", self.tot_read, n),
                ))?,
            },
        };
        self.off_valid += valid_len;

        Ok(valid_len)
    }

    //------------------------------------------------------------------------------

    /// Consumes one character.
    ///
    /// This method will automatically [`pull`](Self::pull) if the content is empty.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<Option<char>> {
        if self.content().is_empty() {
            self.pull()?;
        }

        Ok(self.content().chars().next().inspect(|ch| {
            self.off_consumed += ch.len_utf8();
        }))
    }

    /// Peeks one character.
    ///
    /// This method will automatically [`pull`](Self::pull) if the content is empty.
    pub fn peek(&mut self) -> Result<Option<char>> {
        if self.content().is_empty() {
            self.pull()?;
        }

        Ok(self.content().chars().next())
    }

    /// Consumes one character then peeks the second if the previous call is still [`peek_more`](Self::peek_more),
    /// peeks one character otherwise.
    ///
    /// This method will automatically [`pull_more`](Self::pull_more) if the content is insufficient.
    ///
    /// NOTE: Needs manually let `self.peeked = None`.
    ///
    /** Private method because opaque and unpinned internal offsets. */
    #[inline(always)]
    fn peeking(&mut self) -> Result<Option<char>> {
        if let Some(len) = self.peeked.take() {
            self.off_consumed += len as usize;
        }

        if self.content().is_empty() {
            self.pull_more()?;
        }

        Ok(self.content().chars().next().inspect(|ch| {
            self.peeked = Some(ch.len_utf8() as u8);
        }))
    }

    //------------------------------------------------------------------------------

    /// Consumes one character if `predicate`.
    ///
    /// This method will automatically [`pull`](Self::pull) if the content is empty.
    pub fn take_once<P>(&mut self, predicate: P) -> Result<Option<char>>
    where
        P: Predicate,
    {
        Ok(match self.peek()? {
            None => None,
            Some(ch) => match predicate.predicate(ch) {
                false => None,
                true => {
                    self.off_consumed += ch.len_utf8();
                    Some(ch)
                }
            },
        })
    }

    /// Consumes N..M characters consisting of `predicate`.
    ///
    /// Peeks the first unexpected character additionally, may be `None` if encountered the EOF.
    ///
    /// Returns `Ok((Err(&str), _))` and doesn't consume if the taking times not in `range`.
    ///
    /// This method will automatically [`pull_more`](Self::pull_more) if the content is insufficient.
    pub fn take_times<P, U>(&mut self, predicate: P, range: U) -> Result<(StdResult<&str, &str>, Option<char>)>
    where
        P: Predicate,
        U: RangeBounds<usize>,
    {
        self.peeked = None;

        let mut times = 0;
        let start = self.off_consumed;
        let ch = loop {
            match self.peeking()? {
                None => break None,
                Some(ch) => match match range.end_bound() {
                    Bound::Included(&n) => times <= n,
                    Bound::Excluded(&n) => times < n,
                    Bound::Unbounded => true,
                } && predicate.predicate(ch)
                {
                    false => break Some(ch),
                    true => times += 1,
                },
            }
        };

        let span = unsafe { core::str::from_utf8_unchecked(self.buf.get_unchecked(start..self.off_consumed)) };

        Ok((
            range
                .contains(&times)
                .then_some(span)
                .ok_or(span)
                .inspect_err(|_| self.off_consumed = start),
            ch,
        ))
    }

    /// Consumes X characters consisting of `predicate`.
    ///
    /// Peeks the first unexpected character additionally, may be `None` if encountered the EOF.
    ///
    /// This method will automatically [`pull_more`](Self::pull_more) if the content is insufficient.
    pub fn take_while<P>(&mut self, predicate: P) -> Result<(&str, Option<char>)>
    where
        P: Predicate,
    {
        self.peeked = None;

        let start = self.off_consumed;
        let ch = loop {
            match self.peeking()? {
                None => break None,
                Some(ch) => match predicate.predicate(ch) {
                    false => break Some(ch),
                    true => continue,
                },
            }
        };

        Ok((
            unsafe { core::str::from_utf8_unchecked(self.buf.get_unchecked(start..self.off_consumed)) },
            ch,
        ))
    }

    /// Consumes N characters that precedes `pattern`.
    ///
    /// Returns `Ok(false)` and doesn't consume if did't match or encountered the EOF.
    ///
    /// This method will automatically [`pull_more`](Self::pull_more) if the content is insufficient.
    pub fn matches(&mut self, pattern: &str) -> Result<bool> {
        Ok(if !self.pull_at_least(pattern.len())? {
            false
        } else if unsafe {
            self.buf
                .get_unchecked(self.off_consumed..)
                .get_unchecked(..pattern.len())
        } != pattern.as_bytes()
        {
            false
        } else {
            self.off_consumed += pattern.len();
            true
        })
    }

    /// Consumes X characters until encountered `terminator`.
    ///
    /// The `terminator` is excluded from the result and also marked as consumed.
    ///
    /// Returns `Ok(Err(&str))` and doesn't consume if encountered the EOF.
    ///
    /// This method will automatically [`pull_more`](Self::pull_more) if the content is insufficient.
    ///
    /// # Panics
    ///
    /// Panics if `terminator` is empty.
    pub fn until(&mut self, terminator: &str) -> Result<StdResult<&str, &str>> {
        let start = self.off_consumed;
        let indicator = terminator.as_bytes()[0];

        'outer: loop {
            loop {
                let content = unsafe { self.buf.get_unchecked(self.off_consumed..self.off_valid) };
                match content.iter().position(|b| *b == indicator) {
                    None if self.eof => {
                        break 'outer;
                    }
                    None => {
                        self.off_consumed += content.len();
                        self.pull_more()?;
                    }
                    Some(idx) => {
                        self.off_consumed += idx;
                        break;
                    }
                }
            }

            if !self.pull_at_least(terminator.len())? {
                break 'outer;
            }

            if unsafe {
                self.buf
                    .get_unchecked(self.off_consumed..)
                    .get_unchecked(..terminator.len())
            } == terminator.as_bytes()
            {
                let span = unsafe { core::str::from_utf8_unchecked(self.buf.get_unchecked(start..self.off_consumed)) };

                self.off_consumed += terminator.len();

                return Ok(Ok(span));
            }

            self.off_consumed += 1;
        }

        let span = unsafe { core::str::from_utf8_unchecked(self.buf.get_unchecked(start..self.off_valid)) };

        self.off_consumed = start;

        Ok(Err(span))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan() -> Result<()> {
        let msg = " >< Foo >< Bar >< Baz >< ";
        let mut rdr = Utf8Reader::new(msg.as_bytes());

        let separator = |ch| matches!(ch, ' ' | '>' | '<');

        assert_eq!(rdr.take_while(separator)?, (" >< ", Some('F')));
        assert_eq!(rdr.take_while(alphabetic)?, ("Foo", Some(' ')));

        assert_eq!(rdr.take_while(not!(separator))?, ("", Some(' ')));

        assert_eq!(rdr.until("<>")?, Err(" >< Bar >< Baz >< "));
        assert_eq!(rdr.until("Bar")?, Ok(" >< "));

        Ok(())
    }
}
