use simdutf8::compat::{from_utf8, Utf8Error};
use std::{
    io::{Error, ErrorKind, Read, Result},
    mem,
    ops::Range,
    ptr,
    result::Result as StdResult,
    str::from_utf8_unchecked,
};

#[macro_use]
mod predicates;

pub use predicates::*;

pub struct Utf8Reader<'src, R: Read> {
    src: Source<'src, R>,
    off_consumed: usize,
    tot_consumed: usize,
    peeked: Option<u8>,
    eof: bool,
}

enum Source<'src, R: Read> {
    Borrowed(&'src str),
    Reader {
        rdr: R,
        buf: Box<[u8]>,
        buf_cap: usize,
        tot_read: usize,
        off_read: usize,
        off_valid: usize,
    },
}

/// Uninhabited generic placeholder.
enum __ {}

impl Read for __ {
    fn read(&mut self, _buf: &mut [u8]) -> Result<usize> {
        unreachable!()
    }
}

impl<'src> Utf8Reader<'src, __> {
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &'src str) -> Self {
        Self {
            src: Source::Borrowed(s),
            off_consumed: 0,
            tot_consumed: 0,
            peeked: None,
            eof: true,
        }
    }

    pub fn from_bytes(bytes: &'src [u8]) -> StdResult<Self, Utf8Error> {
        from_utf8(bytes).map(Self::from_str)
    }
}

impl<R: Read> Utf8Reader<'static, R> {
    pub fn from_reader(rdr: R) -> Self {
        Self {
            src: Source::Reader {
                rdr,
                buf: unsafe { Box::new_uninit_slice(Self::INIT_CAP).assume_init() },
                buf_cap: Self::INIT_CAP,
                tot_read: 0,
                off_read: 0,
                off_valid: 0,
            },
            off_consumed: 0,
            tot_consumed: 0,
            peeked: None,
            eof: false,
        }
    }
}

impl<'src, R: Read> Utf8Reader<'src, R> {
    pub const INIT_CAP: usize = 32 * 1024;
    const THRES_ARRANGE: usize = 8 * 1024;

    /// Returns the string of unconsumed, valid UTF-8 bytes.
    #[inline]
    pub fn content(&self) -> &str {
        unsafe {
            match &self.src {
                Source::Borrowed(s) => s.get_unchecked(self.off_consumed..),
                Source::Reader { buf, off_valid, .. } => {
                    from_utf8_unchecked(buf.get_unchecked(self.off_consumed..*off_valid))
                }
            }
        }
    }

    #[inline]
    fn content_behind(&self, span: Range<usize>) -> &str {
        unsafe {
            match &self.src {
                Source::Borrowed(s) => s.get_unchecked(span),
                Source::Reader { buf, .. } => from_utf8_unchecked(buf.get_unchecked(span)),
            }
        }
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
        match &self.src {
            Source::Borrowed(s) => self.off_consumed == s.len(),
            Source::Reader { off_valid, .. } => self.eof && self.off_consumed == *off_valid,
        }
    }

    //------------------------------------------------------------------------------

    /// Pulls no more than [`INIT_CAP`](Self::INIT_CAP) bytes.
    ///
    /*  WARN: The offset of content may NOT be pinned. */
    pub fn pull(&mut self) -> Result<()> {
        let Source::Reader {
            buf,
            buf_cap,
            off_read,
            off_valid,
            ..
        } = &mut self.src
        else {
            return Ok(());
        };

        if *off_read > Self::INIT_CAP + self.off_consumed {
            return Ok(());
        }

        if *off_read > Self::INIT_CAP - Self::THRES_ARRANGE {
            unsafe {
                ptr::copy(
                    buf.as_ptr().add(self.off_consumed),
                    buf.as_ptr() as *mut _,
                    *off_read - self.off_consumed,
                );
            }

            self.tot_consumed += self.off_consumed;

            *off_valid -= self.off_consumed;
            *off_read -= self.off_consumed;

            self.off_consumed = 0;
        }

        *buf_cap = Self::INIT_CAP;

        if *off_read >= Self::INIT_CAP {
            return Ok(());
        }

        self.fetch(Self::pull)
    }

    /// Pulls more bytes, allows the content to grow infinitely.
    ///
    /*  NOTE: The offset of content would be pinned. */
    pub fn pull_more(&mut self) -> Result<()> {
        let Source::Reader {
            buf, buf_cap, off_read, ..
        } = &mut self.src
        else {
            return Ok(());
        };

        if *off_read > *buf_cap * 7 / 8 {
            *buf_cap *= 2;
            if *buf_cap > buf.len() {
                let mut buf_new = unsafe { Box::new_uninit_slice(*buf_cap).assume_init() };
                unsafe { ptr::copy_nonoverlapping(buf.as_ptr(), buf_new.as_mut_ptr(), *off_read) }
                drop(mem::replace(buf, buf_new));
            }
        }

        self.fetch(Self::pull_more)
    }

    /// Pulls more bytes, makes the content has at least `n` bytes.
    ///
    /// Returns `Ok(false)` if encountered the EOF, unable to read such more bytes.
    ///
    /*  NOTE: The offset of content would be pinned.  */
    pub fn pull_at_least(&mut self, n: usize) -> Result<bool> {
        loop {
            let Source::Reader { off_valid, .. } = &self.src else {
                return Ok(self.content().len() >= n);
            };

            match self.off_consumed + n > *off_valid {
                false => return Ok(true),
                true => match !self.eof {
                    false => return Ok(false),
                    true => self.pull_more()?,
                },
            }
        }
    }

    fn fetch(&mut self, rerun: fn(&mut Self) -> Result<()>) -> Result<()> {
        let Source::Reader {
            rdr,
            buf,
            buf_cap,
            tot_read,
            off_read,
            off_valid,
        } = &mut self.src
        else {
            unreachable!()
        };

        let len = unsafe { rdr.read(buf.get_unchecked_mut(*off_read..*buf_cap))? };

        self.eof = len == 0;

        if !self.eof {
            *tot_read += len;
            *off_read += len;
            match self.validate()? {
                true => Ok(()),
                false => rerun(self),
            }
        } else {
            match *off_valid == *off_read {
                true => Ok(()),
                false => Err(Error::new(
                    ErrorKind::UnexpectedEof,
                    "incomplete UTF-8 code point at the end",
                )),
            }
        }
    }

    fn validate(&mut self) -> Result<bool> {
        let Source::Reader {
            buf,
            tot_read,
            off_read,
            off_valid,
            ..
        } = &mut self.src
        else {
            unreachable!()
        };

        if let Err(e) = unsafe { from_utf8(buf.get_unchecked(*off_valid..*off_read)) } {
            match e.error_len() {
                Some(n) => Err(Error::new(
                    ErrorKind::InvalidData,
                    format!("invalid UTF-8 bytes at index {}+{}", tot_read, n),
                )),
                None => Ok(false),
            }
        } else {
            *off_valid = *off_read;
            Ok(true)
        }
    }

    //------------------------------------------------------------------------------

    /// Consumes one character.
    ///
    /// This method will automatically [`pull`](Self::pull) if the content is empty.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<Option<char>> {
        self.pull()?;

        Ok(self.content().chars().next().inspect(|ch| {
            self.off_consumed += ch.len_utf8();
        }))
    }

    /// Peeks one character.
    ///
    /// This method will automatically [`pull`](Self::pull) if the content is empty.
    pub fn peek(&mut self) -> Result<Option<char>> {
        self.pull()?;

        Ok(self.content().chars().next())
    }

    /// Consumes one character then peeks the second if the previous call is still [`peeking`](Self::peeking),
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
        U: URangeBounds,
    {
        self.peeked = None;

        let mut times = 0;
        let start = self.off_consumed;
        let ch = loop {
            match self.peeking()? {
                None => break None,
                Some(ch) => match range.want_more(times) && predicate.predicate(ch) {
                    false => break Some(ch),
                    true => times += 1,
                },
            }
        };

        let span = start..self.off_consumed;

        Ok((
            match range.contains(times) {
                true => Ok(self.content_behind(span)),
                false => {
                    self.off_consumed = start;
                    Err(self.content_behind(span))
                }
            },
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

        Ok((self.content_behind(start..self.off_consumed), ch))
    }

    /// Consumes K characters if matched `pattern`.
    ///
    /// Returns `Ok(None)` and doesn't consume if did't match anything.
    ///
    /// This method will automatically [`pull`](Self::pull) if the content is insufficient.
    pub fn matches<P>(&mut self, pattern: P) -> Result<Option<P::Discriminant>>
    where
        P: Pattern,
    {
        self.pull()?;

        Ok(match self.content().as_bytes().first() {
            None => None,
            Some(&b) => match pattern.indicate(b) {
                None => None,
                Some(len) => {
                    self.pull_at_least(len)?;
                    match pattern.matches(self.content()) {
                        None => None,
                        Some((len, idx)) => {
                            self.off_consumed += len;
                            Some(idx)
                        }
                    }
                }
            },
        })
    }

    /// Consumes X characters until matched `pattern`.
    ///
    /// The `pattern` is excluded from the result and also marked as consumed.
    ///
    /// Returns `Ok(Err(&str))` and doesn't consume if encountered the EOF.
    ///
    /// This method will automatically [`pull_more`](Self::pull_more) if the content is insufficient.
    pub fn until<P>(&mut self, pattern: P) -> Result<StdResult<(&str, P::Discriminant), &str>>
    where
        P: Pattern,
    {
        let start = self.off_consumed;

        'outer: loop {
            let len = loop {
                match self
                    .content()
                    .as_bytes()
                    .iter()
                    .enumerate()
                    .find_map(|(idx, &b)| pattern.indicate(b).map(|len| (idx, len)))
                {
                    None => {
                        self.off_consumed += self.content().len();
                        match !self.eof {
                            false => break 'outer,
                            true => self.pull_more()?,
                        }
                    }
                    Some((idx, len)) => {
                        self.off_consumed += idx;
                        break len;
                    }
                }
            };

            self.pull_at_least(len)?;

            if let Some((len, idx)) = pattern.matches(self.content()) {
                let span = start..self.off_consumed;
                self.off_consumed += len;

                return Ok(Ok((self.content_behind(span), idx)));
            }

            self.off_consumed += 1;
        }

        let span = start..self.off_consumed;
        self.off_consumed = start;

        Ok(Err(self.content_behind(span)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan() -> Result<()> {
        let mut rdr = Utf8Reader::from_str(" >< Foo >< Bar >< Baz >< ");

        let separator = |ch| matches!(ch, ' ' | '>' | '<');

        assert_eq!(rdr.take_while(separator)?, (" >< ", Some('F')));
        assert_eq!(rdr.take_while(alphabetic)?, ("Foo", Some(' ')));

        assert_eq!(rdr.take_while(not!(separator))?, ("", Some(' ')));
        assert_eq!(rdr.take_while(not!(separator, 'b'))?, ("", Some(' ')));

        assert_eq!(rdr.until("<>")?, Err(" >< Bar >< Baz >< "));
        assert_eq!(rdr.until(["Foo", "Bar"])?, Ok((" >< ", "Bar")));

        Ok(())
    }
}
