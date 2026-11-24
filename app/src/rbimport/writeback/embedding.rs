//! The observed v7 .3EX is a MessagePack map containing audio features. Only this
//! complete, bounded shape is recognized. Its d2 identifies the track; d11
//! matches native tempo in the public samples that have a beat grid.
struct Input<'a>(&'a [u8]);

impl<'a> Input<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let (bytes, rest) = self.0.split_at_checked(n)?;
        self.0 = rest;
        Some(bytes)
    }

    fn byte(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }

    fn text(&mut self) -> Option<&'a [u8]> {
        let n = match self.byte()? {
            m @ 0xa0..=0xbf => usize::from(m - 0xa0),
            0xd9 => usize::from(self.byte()?),
            _ => return None,
        };
        self.take(n)
    }

    fn array_len(&mut self) -> Option<usize> {
        match self.byte()? {
            m @ 0x90..=0x9f => Some(usize::from(m - 0x90)),
            0xdc => Some(usize::from(u16::from_be_bytes(
                self.take(2)?.try_into().ok()?,
            ))),
            _ => None,
        }
    }

    fn float(&mut self) -> Option<()> {
        let finite = match self.byte()? {
            0xca => f32::from_be_bytes(self.take(4)?.try_into().ok()?).is_finite(),
            0xcb => f64::from_be_bytes(self.take(8)?.try_into().ok()?).is_finite(),
            _ => return None,
        };
        finite.then_some(())
    }

    fn vector(&mut self, width: usize) -> Option<()> {
        (self.array_len()? == width).then_some(())?;
        for _ in 0..width {
            self.float()?;
        }
        Some(())
    }

    fn embedding_owner(&mut self) -> Option<&'a [u8]> {
        (self.byte()? == 0x81 && self.text()? == b"embedding" && self.byte()? == 0x8b)
            .then_some(())?;
        let mut seen = 0u16;
        let mut owner = None;
        for _ in 0..11 {
            let field = match self.text()? {
                [b'd', n @ b'2'..=b'9'] => n - b'0',
                b"d10" => 10,
                b"d11" => 11,
                b"d12" => 12,
                _ => return None,
            };
            let bit = 1u16 << field;
            (seen & bit == 0).then_some(())?;
            seen |= bit;
            match field {
                2 => {
                    let id = self.text()?;
                    (!id.is_empty() && id.len() <= 20 && id.iter().all(u8::is_ascii_digit))
                        .then_some(())?;
                    owner = Some(id);
                }
                3 => {
                    let hash = self.text()?;
                    (hash.len() == 32 && hash.iter().all(u8::is_ascii_hexdigit)).then_some(())?;
                }
                4 | 8 | 11 | 12 => self.float()?,
                5 => {
                    let n = self.array_len()?;
                    (n > 0 && n <= self.0.len() / (64 * 5)).then_some(())?;
                    for _ in 0..n {
                        self.vector(64)?;
                    }
                }
                6 => self.vector(64)?,
                7 => self.vector(2)?,
                9 | 10 => {
                    (self.byte()? <= 0x7f).then_some(())?;
                }
                _ => return None,
            }
        }
        self.0.is_empty().then_some(())?;
        owner
    }
}

pub(super) fn content_id(bytes: &[u8]) -> Option<&[u8]> {
    (bytes.len() <= 1024 * 1024).then_some(())?;
    Input(bytes).embedding_owner()
}

pub(super) fn is_known(bytes: &[u8]) -> bool {
    content_id(bytes).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_complete_single_embedding_without_extra_keys_is_accepted() {
        let mut b = b"\x81\xa9embedding\x8b".to_vec();
        b.extend(b"\xa2d2\xa3123\xa2d3\xd9\x20");
        b.extend([b'0'; 32]);
        let float = [0xca, 0, 0, 0, 0];
        for n in [4, 5, 6, 7, 8, 9, 10, 11, 12] {
            let key = format!("d{n}");
            b.push(0xa0 + key.len() as u8);
            b.extend(key.bytes());
            match n {
                5 | 6 => {
                    if n == 5 {
                        b.push(0x91);
                    }
                    b.extend([0xdc, 0, 64]);
                    for _ in 0..64 {
                        b.extend(float);
                    }
                }
                7 => {
                    b.push(0x92);
                    b.extend(float);
                    b.extend(float);
                }
                9 | 10 => b.push(0),
                _ => b.extend(float),
            }
        }
        assert_eq!(content_id(&b), Some(b"123".as_slice()));
        assert!(!is_known(&b[..b.len() - 1]));
        let mut duplicate = b.clone();
        let key = duplicate.windows(3).position(|w| w == b"d12").unwrap();
        duplicate[key + 2] = b'1';
        assert!(!is_known(&duplicate));
        b.extend_from_slice(b"PCO2");
        assert!(!is_known(&b));
        assert!(!is_known(b"\x81\xa9embedding\x80"));
    }
}
