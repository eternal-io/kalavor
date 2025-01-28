use simdutf8::compat::from_utf8;
use std::{
    io::{Error, ErrorKind, Read, Result},
    ptr,
};

pub struct Utf8Reader<R: Read> {
    src: R,
    buf: Vec<u8>,
    buf_cap: usize,
    off_consumed: usize,
    off_valid: usize,
    off_raw: usize,
    tot_read: usize,
    peeked: bool,
    eof: bool,
}

impl<R: Read> Utf8Reader<R> {
    pub const INIT_CAP: usize = 16 * 1024;
    const THRES_SHRINK: usize = 2 * 1024;
    const THRES_EXTEND: usize = 4 * 1024;

    pub fn new(src: R) -> Self {
        Self {
            src,
            buf: Vec::with_capacity(Self::INIT_CAP),
            buf_cap: Self::INIT_CAP,
            off_consumed: 0,
            off_valid: 0,
            off_raw: 0,
            tot_read: 0,
            peeked: false,
            eof: false,
        }
    }

    /// Returns `true` if all the bytes are consumed and unable to read more.
    pub fn eof(&self) -> bool {
        self.eof && self.off_consumed == self.off_raw
    }

    /// Returns the string of unconsumed, valid UTF-8 bytes.
    pub fn content(&self) -> &str {
        unsafe { core::str::from_utf8_unchecked(&self.buf[self.off_consumed..self.off_valid]) }
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
        self.peeked = true;

        if self.content().is_empty() {
            self.pull()?;
        }

        Ok(self.content().chars().next())
    }

    /// Returns the first character of the content, consumes it if the previous call is still [`peeking`](Self::peeking) or [`peek`](Self::peek).
    ///
    /// The continuous state would be broken by the call of [`consume`](Self::consume) or [`next`](Self::next).
    ///
    /// This method will automatically [`pull`](Self::pull) if the content is empty.
    pub fn peeking(&mut self) -> Result<Option<char>> {
        self.peeked = true;

        if self.content().is_empty() {
            self.pull()?;
        }

        Ok(self.content().chars().next().inspect(|ch| {
            if self.peeked {
                self.off_consumed += ch.len_utf8();
            }
        }))
    }

    /// Pulls no more than [`Self::INIT_CAP`] bytes.
    pub fn pull(&mut self) -> Result<()> {
        if self.off_raw - self.off_consumed > Self::INIT_CAP {
            return Ok(());
        }

        if self.off_raw + Self::THRES_SHRINK > Self::INIT_CAP {
            unsafe {
                ptr::copy(
                    self.buf.as_ptr().add(self.off_consumed),
                    self.buf.as_ptr() as *mut _,
                    self.off_raw - self.off_consumed,
                )
            }

            self.off_raw -= self.off_consumed;
            self.off_valid -= self.off_consumed;
            self.off_consumed = 0;
        }

        if self.off_raw >= Self::INIT_CAP {
            return Ok(());
        }

        let len = self.src.read(&mut self.buf[self.off_raw..Self::INIT_CAP])?;
        self.eof = len == 0;
        self.off_raw += len;
        self.tot_read += len;

        self.validate()
    }

    /// Pulls more than [`Self::INIT_CAP`] bytes.
    pub fn pull_more(&mut self) -> Result<()> {
        if self.buf_cap - self.off_raw < Self::THRES_EXTEND {
            self.buf.reserve(self.buf_cap);
            self.buf_cap *= 2;
            unsafe { self.buf.set_len(self.buf_cap) }
        }

        let len = self.src.read(&mut self.buf[self.off_raw..self.buf_cap])?;
        self.eof = len == 0;
        self.off_raw += len;
        self.tot_read += len;

        self.validate()
    }

    fn validate(&mut self) -> Result<()> {
        self.off_valid += match from_utf8(&self.buf[self.off_valid..self.off_raw]) {
            Ok(s) => s.len(),
            Err(e) => match e.error_len() {
                None => e.valid_up_to(),
                Some(_) => Err(Error::new(
                    ErrorKind::InvalidData,
                    format!("invalid UTF-8 byte at index {}", self.tot_read),
                ))?,
            },
        };

        Ok(())
    }
}
