use simdutf8::compat::{from_utf8, Utf8Error};
use std::{
    io::{self, Error, ErrorKind, Read},
    mem,
    ops::Range,
    ptr,
    str::from_utf8_unchecked,
};

#[macro_use]
mod predicates;

pub use predicates::*;

#[cfg(test)]
mod tests;

pub struct Utf8Reader<'src, R: Read> {
    src: Source<'src, R>,
    off_consumed: usize,
    tot_consumed: usize,
    peeked: Option<u8>,
    eof: bool,
}

enum Source<'src, R: Read> {
    Borrowed {
        slice: &'src str,
    },
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
enum Slice {}

impl Read for Slice {
    fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        unreachable!()
    }
}

impl<'src> Utf8Reader<'src, Slice> {
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(slice: &'src str) -> Self {
        Self {
            src: Source::Borrowed { slice },
            off_consumed: 0,
            tot_consumed: 0,
            peeked: None,
            eof: true,
        }
    }

    pub fn from_bytes(bytes: &'src [u8]) -> Result<Self, Utf8Error> {
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
    const INIT_CAP: usize = 32 * 1024;
    const THRES_REARRANGE: usize = 8 * 1024;

    /// Returns the slice of unconsumed, valid UTF-8 bytes.
    #[inline]
    pub fn content(&self) -> &str {
        unsafe {
            match &self.src {
                Source::Borrowed { slice } => slice.get_unchecked(self.off_consumed..),
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
                Source::Borrowed { slice } => slice.get_unchecked(span),
                Source::Reader { buf, .. } => from_utf8_unchecked(buf.get_unchecked(span)),
            }
        }
    }

    /// Marks the leading `n` bytes of the content as consumed, they will disappear in the future content.
    ///
    /// # Panics
    ///
    /// Panics if the `n`th byte is not at a UTF-8 character boundary.
    pub fn bump(&mut self, n: usize) {
        if !self.content().is_char_boundary(n) {
            panic!("{} is not at a UTF-8 character boundary", n)
        }

        self.off_consumed += n;
    }

    /// Returns the count of totally consumed bytes.
    pub fn consumed(&self) -> usize {
        self.tot_consumed + self.off_consumed
    }

    /// Returns `true` if all bytes are consumed and encountered the EOF.
    pub fn exhausted(&self) -> bool {
        match &self.src {
            Source::Borrowed { slice } => self.off_consumed == slice.len(),
            Source::Reader { off_valid, .. } => self.eof && self.off_consumed == *off_valid,
        }
    }

    //------------------------------------------------------------------------------

    /// Pull bytes if available content is less than 8 KiB.
    ///
    /*  WARN: The offset of content may NOT be pinned. */
    pub fn pull(&mut self) -> io::Result<()> {
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

        /**************************************************************************************************
         *            THRES_REARRANGE       INIT_CAP                                                      *
         *                          ↓       ↓                                                             *
         *  +-------+-------+-------+-------+                                                             *
         *  |       |       |       |<<<<<<<| buffer                                                      *
         *  +-------+-------+-------+-------+                                                             *
         *                          '                                                                     *
         *                         ^~~~~~~~~$ Next first `if` captured (`content().len() > THRES`),       *
         *                          '         the span can be shifted/expanded arbitrary.                 *
         *                          '                                                                     *
         *                     ^~~~~'~~$      Next second `if` captured (`content().len() <= THRES`),     *
         *                    ^~~~~~'~$       it's guaranteed no overlap to rearrange the buffer,         *
         *                   ^~~~~~~'$        and `buf_cap` can reset safely.                             *
         *                          '                                                                     *
         *                  ^~~~~~~~$         <- "worst" case.                                            *
         *                         X'                                                                     *
         *                          '                                                                     *
         *  ^~~~~~~~$               '         After rearrangement.                                        *
         *                                                                                                *
         *  where the span is `off_consumed..off_read`.                                                   *
         **************************************************************************************************/

        if *off_read - self.off_consumed > Self::THRES_REARRANGE {
            return Ok(());
        }

        if *off_read >= Self::INIT_CAP - Self::THRES_REARRANGE {
            unsafe {
                ptr::copy_nonoverlapping(
                    buf.as_ptr().add(self.off_consumed),
                    buf.as_ptr() as *mut _,
                    *off_read - self.off_consumed,
                );
            }

            self.tot_consumed += self.off_consumed;

            *off_valid -= self.off_consumed;
            *off_read -= self.off_consumed;

            self.off_consumed = 0;

            *buf_cap = Self::INIT_CAP;
        }

        self.fetch(Self::pull)
    }

    /// Pull more bytes, allows the content to grow infinitely.
    ///
    /*  NOTE: The offset of content would be pinned. */
    pub fn pull_more(&mut self) -> io::Result<()> {
        let Source::Reader {
            buf, buf_cap, off_read, ..
        } = &mut self.src
        else {
            return Ok(());
        };

        fn m7d8(n: usize) -> usize {
            (n >> 1) + (n >> 2) + (n >> 3)
        }

        if *off_read > m7d8(*buf_cap) {
            *buf_cap <<= 1;
            if *buf_cap > buf.len() {
                let mut buf_new = unsafe { Box::new_uninit_slice(*buf_cap).assume_init() };
                unsafe { ptr::copy_nonoverlapping(buf.as_ptr(), buf_new.as_mut_ptr(), *off_read) }
                drop(mem::replace(buf, buf_new));
            }
        }

        self.fetch(Self::pull_more)
    }

    /// Pull more bytes, makes the content has at least `n` bytes.
    ///
    /// Returns `Ok(false)` if encountered the EOF, unable to read such more bytes.
    ///
    /*  NOTE: The offset of content would be pinned.  */
    pub fn pull_at_least(&mut self, n: usize) -> io::Result<bool> {
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

    fn fetch(&mut self, rerun: fn(&mut Self) -> io::Result<()>) -> io::Result<()> {
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
                    format!("incomplete UTF-8 code point at the end ({})", *off_valid),
                )),
            }
        }
    }

    fn validate(&mut self) -> io::Result<bool> {
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

    /// Consume one character.
    ///
    /// This method will automatically [`pull`](Self::pull) if the content is empty.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> io::Result<Option<char>> {
        self.pull()?;

        Ok(self.content().chars().next().inspect(|ch| {
            self.off_consumed += ch.len_utf8();
        }))
    }

    /// Peeks one character.
    ///
    /// This method will automatically [`pull`](Self::pull) if the content is empty.
    pub fn peek(&mut self) -> io::Result<Option<char>> {
        self.pull()?;

        Ok(self.content().chars().next())
    }

    /// As same as the [`next`](Self::next), but [`pull_more`](Self::pull_more) instead.
    ///
    /** Private method because opaque and unpinned internal offsets. */
    #[inline(always)]
    fn nexting(&mut self) -> io::Result<Option<char>> {
        if self.content().is_empty() {
            self.pull_more()?;
        }

        Ok(self.content().chars().next().inspect(|ch| {
            self.off_consumed += ch.len_utf8();
        }))
    }

    /// Consume one character then peeks the second if the previous call is still [`peeking`](Self::peeking),
    /// peeks one character otherwise.
    ///
    /// This method will automatically [`pull_more`](Self::pull_more) if the content is insufficient.
    ///
    /// NOTE: Needs manually let `self.peeked = None`.
    ///
    /** Private method because opaque and unpinned internal offsets. */
    #[inline(always)]
    fn peeking(&mut self) -> io::Result<Option<char>> {
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

    /// Consume N characters.
    ///
    /// Returns `Ok(Err(_))` if encountered the EOF.
    ///
    /// This method will automatically [`pull_more`](Self::pull_more) if the content is insufficient.
    pub fn take(&mut self, n_char: usize) -> io::Result<Result<&str, &str>> {
        let start = self.off_consumed;

        for _ in 0..n_char {
            if self.nexting()?.is_none() {
                return Ok(Err(self.content_behind(start..self.off_consumed)));
            }
        }

        Ok(Ok(self.content_behind(start..self.off_consumed)))
    }

    /// Consume one character if `predicate`.
    ///
    /// Returns `Ok(None)` if encountered the EOF.
    ///
    /// This method will automatically [`pull`](Self::pull) if the content is empty.
    pub fn take_once<P>(&mut self, predicate: P) -> io::Result<Option<char>>
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

    /// Consume N..M characters consisting of `predicate`.
    ///
    /// Peeks the first unexpected character additionally, may be `None` if encountered the EOF.
    ///
    /// Returns `Ok(Err(_))` and doesn't consume if the taking times not in `range`.
    ///
    /// This method will automatically [`pull_more`](Self::pull_more) if the content is insufficient.
    #[allow(clippy::type_complexity)]
    pub fn take_times<P, U>(
        &mut self,
        predicate: P,
        range: U,
    ) -> io::Result<Result<(&str, Option<char>), (&str, Option<char>)>>
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

        Ok(match range.contains(times) {
            true => Ok((self.content_behind(span), ch)),
            false => {
                self.off_consumed = start;
                Err((self.content_behind(span), ch))
            }
        })
    }

    /// Consume X characters consisting of `predicate`.
    ///
    /// Peeks the first unexpected character additionally, may be `None` if encountered the EOF.
    ///
    /// This method will automatically [`pull_more`](Self::pull_more) if the content is insufficient.
    pub fn take_while<P>(&mut self, predicate: P) -> io::Result<(&str, Option<char>)>
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

    /// Consume K characters if matched `pattern`.
    ///
    /// Returns `Ok(None)` and doesn't consume if did't match anything.
    ///
    /// This method will automatically [`pull`](Self::pull) if the content is insufficient.
    pub fn matches<P>(&mut self, pattern: P) -> io::Result<Option<P::Discriminant>>
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

    /// Consume X characters until encountered `predicate`.
    ///
    /// The `predicate` is excluded from the result and also marked as consumed.
    ///
    /// Returns `Ok(Err(_))` and doesn't consume if encountered the EOF.
    ///
    /// This method will automatically [`pull_more`](Self::pull_more) if the content is insufficient.
    pub fn skim_till<P>(&mut self, predicate: P) -> io::Result<Result<(&str, char), &str>>
    where
        P: Predicate,
    {
        self.peeked = None;

        let start = self.off_consumed;
        let ch = loop {
            match self.peeking()? {
                None => break None,
                Some(ch) => match !predicate.predicate(ch) {
                    false => break Some(ch),
                    true => continue,
                },
            }
        };

        let spanned = self.content_behind(start..self.off_consumed);

        Ok(match ch {
            Some(ch) => Ok((spanned, ch)),
            None => Err(spanned),
        })
    }

    /// Consume X characters until encountered `pattern`.
    ///
    /// The `pattern` is excluded from the result and also marked as consumed.
    ///
    /// Returns `Ok(Err(_))` and doesn't consume if encountered the EOF.
    ///
    /// This method will automatically [`pull_more`](Self::pull_more) if the content is insufficient.
    pub fn skim_until<P>(&mut self, pattern: P) -> io::Result<Result<(&str, P::Discriminant), &str>>
    where
        P: Pattern,
    {
        let start = self.off_consumed;
        let (span, discr) = 'outer: loop {
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
                            false => break 'outer (start..self.off_consumed, None),
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

            if let Some((len, discr)) = pattern.matches(self.content()) {
                let span = start..self.off_consumed;
                self.off_consumed += len;
                break (span, Some(discr));
            }

            self.off_consumed += 1;
        };

        Ok(match discr {
            Some(discr) => Ok((self.content_behind(span), discr)),
            None => {
                self.off_consumed = start;
                Err(self.content_behind(span))
            }
        })
    }

    /// Deprecate X characters until encountered `predicate`.
    ///
    /// Peeks the first encountered character additionally, may be `None` if encountered the EOF.
    pub fn skip_till<P>(&mut self, predicate: P) -> io::Result<Option<char>>
    where
        P: Predicate,
    {
        self.peeked = None;

        Ok(loop {
            match self.peeking()? {
                None => break None,
                Some(ch) => match !predicate.predicate(ch) {
                    false => break Some(ch),
                    true => continue,
                },
            }
        })
    }

    /// Deprecate X characters until encountered `pattern`.
    ///
    /// Peeks the first encountered sub-pattern additionally, may be `None` if encountered the EOF.
    pub fn skip_until<P>(&mut self, pattern: P) -> io::Result<Option<P::Discriminant>>
    where
        P: Pattern,
    {
        Ok('outer: loop {
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
                            false => break 'outer None,
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

            if let Some((len, discr)) = pattern.matches(self.content()) {
                self.off_consumed += len;
                break Some(discr);
            }

            self.off_consumed += 1;
        })
    }
}
