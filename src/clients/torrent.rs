//! Bounded metainfo validation without exposing tracker URLs or passkeys.

use sha1::{Digest, Sha1};

/// Hash the original bencoded info dictionary, never a re-encoded copy.
pub(crate) fn info_hash(bytes: &[u8]) -> Option<String> {
    let mut parser = Parser { bytes, offset: 0 };
    if parser.take()? != b'd' {
        return None;
    }
    let mut info = None;
    while parser.peek()? != b'e' {
        let key = parser.string()?;
        let start = parser.offset;
        if key == b"info" && (info.is_some() || parser.peek()? != b'd') {
            return None;
        }
        parser.value(0)?;
        if key == b"info" {
            if parser.offset - start <= 2 {
                return None;
            }
            info = Some(&bytes[start..parser.offset]);
        }
    }
    parser.take()?;
    if parser.offset != bytes.len() {
        return None;
    }
    Some(format!("{:x}", Sha1::digest(info?)))
}

struct Parser<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.offset).copied()
    }

    fn take(&mut self) -> Option<u8> {
        let byte = self.peek()?;
        self.offset += 1;
        Some(byte)
    }

    fn string(&mut self) -> Option<&'a [u8]> {
        let start = self.offset;
        while self.peek()?.is_ascii_digit() {
            self.offset += 1;
        }
        let digits = &self.bytes[start..self.offset];
        if digits.is_empty() || (digits.len() > 1 && digits[0] == b'0') || self.take()? != b':' {
            return None;
        }
        let length: usize = std::str::from_utf8(digits).ok()?.parse().ok()?;
        let end = self.offset.checked_add(length)?;
        let result = self.bytes.get(self.offset..end)?;
        self.offset = end;
        Some(result)
    }

    fn value(&mut self, depth: usize) -> Option<()> {
        if depth >= 64 {
            return None;
        }
        match self.peek()? {
            b'd' | b'l' => {
                let dictionary = self.take()? == b'd';
                while self.peek()? != b'e' {
                    if dictionary {
                        self.string()?;
                    }
                    self.value(depth + 1)?;
                }
                self.take()?;
            }
            b'i' => {
                self.take()?;
                let start = self.offset;
                if self.peek()? == b'-' {
                    self.take()?;
                }
                let digits = self.offset;
                while self.peek()?.is_ascii_digit() {
                    self.take()?;
                }
                let number = &self.bytes[digits..self.offset];
                if number.is_empty()
                    || (number.len() > 1 && number[0] == b'0')
                    || (&self.bytes[start..self.offset] == b"-0")
                    || self.take()? != b'e'
                {
                    return None;
                }
            }
            b'0'..=b'9' => {
                self.string()?;
            }
            _ => return None,
        }
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_original_info_and_rejects_non_metainfo_or_truncated_responses() {
        let info =
            b"d6:lengthi1e4:name4:test12:piece lengthi16384e6:pieces20:aaaaaaaaaaaaaaaaaaaae";
        let bytes = [b"d4:info".as_slice(), info, b"e"].concat();
        assert_eq!(info_hash(&bytes), Some(format!("{:x}", Sha1::digest(info))));
        for invalid in [
            b"<html>blocked</html>".as_slice(),
            br#"{"code":"1","message":"secret"}"#,
            b"de",
            b"d4:infodee",
            b"d4:info4:texte",
            b"d4:infod4:name3:ab",
            b"d4:infod1:ai01eee",
            b"d4:infod1:ai-0eee",
            b"d4:infod1:ai1eeeextra",
            b"d4:infod1:ai1ee4:infod1:bi2eee",
        ] {
            assert!(info_hash(invalid).is_none());
        }
        for end in 0..bytes.len() {
            assert!(info_hash(&bytes[..end]).is_none());
        }
        let nested = [b"d4:info".as_slice(), &[b'l'; 100], &[b'e'; 101]].concat();
        assert!(info_hash(&nested).is_none());
    }
}
