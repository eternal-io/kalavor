use simdutf8::compat::from_utf8;
use std::{
    io::{Error, ErrorKind, Read, Result},
    ptr,
};

pub struct Utf8Reader<R: Read> {
    src: R,
    buf: Vec<u8>,
    buf_cap: usize,
    tot_read: usize,
    tot_consumed: usize,
    off_consumed: usize,
    off_valid: usize,
    off_raw: usize,
    peeked: bool,
    eoff: bool,
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
            peeked: false,
            eoff: false,
        }
    }

    /// Returns `true` if encountered the EOF and all bytes are consumed.
    pub fn eof(&self) -> bool {
        self.eoff && self.off_consumed == self.off_valid
    }

    /// Returns the count of totally consumed bytes.
    pub fn consumed(&self) -> usize {
        self.tot_consumed + self.off_consumed
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
        self.peeked = false;

        if !self.content().is_char_boundary(n) {
            panic!("{} is not at a UTF-8 character boundary", n)
        }

        self.off_consumed += n;
    }

    /// Returns the first character of the content, then consumes it.
    ///
    /// This method will automatically [`pull`](Self::pull) if the content is empty.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<Option<char>> {
        self.peeked = false;

        if self.content().is_empty() {
            self.pull()?;
        }

        Ok(self.content().chars().next().inspect(|ch| {
            self.off_consumed += ch.len_utf8();
        }))
    }

    /// Returns the first character of the content.
    ///
    /// This method will automatically [`pull`](Self::pull) if the content is empty.
    pub fn peek(&mut self) -> Result<Option<char>> {
        if self.content().is_empty() {
            self.pull()?;
        }

        self.peeked = true;

        Ok(self.content().chars().next())
    }

    /// Returns the first character of the content, consumes it if the previous call is still
    /// [`peek`](Self::peek), [`peeking`](Self::peeking) or [`peeking_more`](Self::peeking_more).
    ///
    /// The continuous state would be broken by the call of [`next`](Self::next) or [`consume`](Self::consume).
    ///
    /// This method will automatically [`pull`](Self::pull) if the content is empty.
    pub fn peeking(&mut self) -> Result<Option<char>> {
        if self.content().is_empty() {
            self.pull()?;
        }

        let opt = self.content().chars().next().inspect(|ch| {
            if self.peeked {
                self.off_consumed += ch.len_utf8();
            }
        });

        self.peeked = true;

        Ok(opt)
    }

    /// Returns the first character of the content, consumes it if the previous call is still
    /// [`peek`](Self::peek), [`peeking`](Self::peeking) or [`peeking_more`](Self::peeking_more).
    ///
    /// The continuous state would be broken by the call of [`next`](Self::next) or [`consume`](Self::consume).
    ///
    /// This method will automatically [`pull_more`](Self::pull_more) if the content is insufficient.
    ///
    /// **Private method because `off_consumed` is invisible.**
    #[inline(always)]
    fn peeking_more(&mut self) -> Result<Option<char>> {
        if self.content().is_empty() {
            self.pull_more()?;
        }

        let opt = self.content().chars().next().inspect(|ch| {
            if self.peeked {
                self.off_consumed += ch.len_utf8();
            }
        });

        self.peeked = true;

        Ok(opt)
    }

    /// Scans a piece of content consisting of `predicate`.
    ///
    /// Returns the first unexpected character additionally, may be `None` if encountered the EOF.
    ///
    /// This method will automatically [`pull_more`](Self::pull_more) if the content is insufficient.
    pub fn scan(&mut self, predicate: impl Fn(char) -> bool) -> Result<(&str, Option<char>)> {
        let start = self.off_consumed;
        let ch = loop {
            if let Some(ch) = self.peeking_more()? {
                if !predicate(ch) {
                    break Some(ch);
                }
            } else {
                break None;
            }
        };

        Ok((
            unsafe { core::str::from_utf8_unchecked(self.buf.get_unchecked(start..self.off_consumed)) },
            ch,
        ))
    }

    /// Scans a piece of content ends with `terminator`. The `terminator` is excluded from the result and marked as consumed.
    ///
    /// Returns `Ok(Err(&str))` if encountered the EOF.
    ///
    /// This method will automatically [`pull_more`](Self::pull_more) if the content is insufficient.
    pub fn scan_until(&mut self, terminator: &str) -> Result<std::result::Result<&str, &str>> {
        let start = self.off_consumed;
        let indicator = terminator.as_bytes()[0];

        'outer: loop {
            loop {
                let content = unsafe { self.buf.get_unchecked(self.off_consumed..self.off_valid) };
                match content.iter().position(|b| *b == indicator) {
                    None if self.eoff => {
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
                    == terminator.as_bytes()
            } {
                let span = unsafe { core::str::from_utf8_unchecked(self.buf.get_unchecked(start..self.off_consumed)) };

                self.off_consumed += terminator.len();

                return Ok(Ok(span));
            }
        }

        self.off_consumed = self.off_valid;

        Ok(Err(unsafe {
            core::str::from_utf8_unchecked(self.buf.get_unchecked(start..self.off_consumed))
        }))
    }

    /// Pulls no more than [`Self::INIT_CAP`] bytes.
    /*
        NOTE: Contents MAY be moved.
    */
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

    /// Pulls more bytes, allowing the content to grow infinitely but slowly (at most [`Self::GROW_CAP`] bytes for each call).
    /*
        NOTE: Contents would NOT be moved.
    */
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
    /*
        NOTE: Contents would NOT be moved.
    */
    pub fn pull_at_least(&mut self, n: usize) -> Result<bool> {
        loop {
            match self.content().len() < n {
                false => return Ok(true),
                true => match !self.eoff {
                    false => return Ok(false),
                    true => self.pull_more()?,
                },
            }
        }
    }

    fn fetch(&mut self, rerun: fn(&mut Self) -> Result<()>) -> Result<()> {
        let len = unsafe { self.src.read(self.buf.get_unchecked_mut(self.off_raw..self.buf_cap))? };

        self.eoff = len == 0;

        if !self.eoff {
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
}
