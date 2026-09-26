//! DEFLATE decompression (RFC 1951) for FlateDecode streams.
//! Classic puff-style canonical Huffman decoding, ~200 lines, no deps.

#[derive(Debug)]
struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    bit: u32,
    acc: u32,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        BitReader { data, pos: 0, bit: 0, acc: 0 }
    }
    fn need(&mut self, n: u32) -> Result<(), String> {
        while self.bit < n {
            let byte = *self
                .data
                .get(self.pos)
                .ok_or_else(|| "truncated deflate stream".to_string())?;
            self.acc |= u32::from(byte) << self.bit;
            self.pos += 1;
            self.bit += 8;
        }
        Ok(())
    }
    fn bits(&mut self, n: u32) -> Result<u32, String> {
        if n == 0 {
            return Ok(0);
        }
        self.need(n)?;
        let v = self.acc & ((1 << n) - 1);
        self.acc >>= n;
        self.bit -= n;
        Ok(v)
    }
    fn align(&mut self) {
        self.acc >>= self.bit % 8;
        self.bit -= self.bit % 8;
    }
    fn take_bytes(&mut self, len: usize, out: &mut Vec<u8>) -> Result<(), String> {
        while self.bit >= 8 {
            out.push((self.acc & 0xff) as u8);
            self.acc >>= 8;
            self.bit -= 8;
        }
        let end = self.pos + len;
        if end > self.data.len() {
            return Err("truncated stored block".into());
        }
        out.extend_from_slice(&self.data[self.pos..end]);
        self.pos = end;
        Ok(())
    }
}

struct Huffman {
    counts: [u16; 16],
    symbols: Vec<u16>,
}

impl Huffman {
    fn build(lengths: &[u8]) -> Result<Huffman, String> {
        let mut counts = [0u16; 16];
        for &l in lengths {
            counts[l as usize] += 1;
        }
        counts[0] = 0;
        let mut left = 1i32;
        for &count in counts.iter().take(15).skip(1) {
            left <<= 1;
            left -= i32::from(count);
            if left < 0 {
                return Err("over-subscribed huffman code".into());
            }
        }
        let mut order: Vec<usize> = (0..lengths.len()).filter(|&i| lengths[i] != 0).collect();
        order.sort_by_key(|&i| (lengths[i], i));
        let symbols = order.into_iter().map(|i| i as u16).collect();
        Ok(Huffman { counts, symbols })
    }
    fn decode(&self, br: &mut BitReader) -> Result<u16, String> {
        let mut code = 0i32;
        let mut first = 0i32;
        let mut index = 0i32;
        for len in 1..16 {
            code |= br.bits(1)? as i32;
            let count = self.counts[len] as i32;
            if code - first < count {
                return Ok(self.symbols[(index + (code - first)) as usize]);
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        Err("invalid huffman code".into())
    }
}

const LEN_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115,
    131, 163, 195, 227, 258,
];
const LEN_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12,
    13, 13,
];
const CL_ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

/// Inflate a raw DEFLATE stream (ignores trailing bytes after bfinal).
pub fn inflate(data: &[u8]) -> Result<Vec<u8>, String> {
    inflate_consumed(data).map(|(out, _)| out)
}

/// Like [`inflate`] but also returns how many input bytes the deflate
/// stream consumed (header-to-bfinal).
pub fn inflate_consumed(data: &[u8]) -> Result<(Vec<u8>, usize), String> {
    let mut br = BitReader::new(data);
    let mut out = Vec::new();
    loop {
        let bfinal = br.bits(1)?;
        let btype = br.bits(2)?;
        match btype {
            0 => {
                br.align();
                let mut hdr = Vec::new();
                br.take_bytes(4, &mut hdr)?;
                let len = u16::from_le_bytes([hdr[0], hdr[1]]) as usize;
                let nlen = u16::from_le_bytes([hdr[2], hdr[3]]) as usize;
                if len ^ 0xffff != nlen {
                    return Err("stored block length mismatch".into());
                }
                br.take_bytes(len, &mut out)?;
            }
            1 => {
                let mut lit = [0u8; 288];
                for (i, l) in lit.iter_mut().enumerate() {
                    *l = match i {
                        0..=143 => 8,
                        144..=255 => 9,
                        256..=279 => 7,
                        _ => 8,
                    };
                }
                let lit_t = Huffman::build(&lit)?;
                let dist_t = Huffman::build(&[5u8; 30])?;
                inflate_block(&mut br, &mut out, &lit_t, &dist_t)?;
            }
            2 => {
                let hlit = br.bits(5)? as usize + 257;
                let hdist = br.bits(5)? as usize + 1;
                let hclen = br.bits(4)? as usize + 4;
                let mut cl_lengths = [0u8; 19];
                for &o in CL_ORDER.iter().take(hclen) {
                    cl_lengths[o] = br.bits(3)? as u8;
                }
                let cl = Huffman::build(&cl_lengths)?;
                let mut lengths = vec![0u8; hlit + hdist];
                let mut i = 0usize;
                while i < lengths.len() {
                    let sym = cl.decode(&mut br)?;
                    match sym {
                        0..=15 => {
                            lengths[i] = sym as u8;
                            i += 1;
                        }
                        16 => {
                            if i == 0 {
                                return Err("repeat with no previous".into());
                            }
                            let prev = lengths[i - 1];
                            let n = 3 + br.bits(2)? as usize;
                            if i + n > lengths.len() {
                                return Err("length overflow".into());
                            }
                            for _ in 0..n {
                                lengths[i] = prev;
                                i += 1;
                            }
                        }
                        17 | 18 => {
                            let n = if sym == 17 {
                                3 + br.bits(3)? as usize
                            } else {
                                11 + br.bits(7)? as usize
                            };
                            if i + n > lengths.len() {
                                return Err("length overflow".into());
                            }
                            i += n;
                        }
                        _ => return Err("bad code-length symbol".into()),
                    }
                }
                let lit = Huffman::build(&lengths[..hlit])?;
                let dist = Huffman::build(&lengths[hlit..hlit + hdist])?;
                inflate_block(&mut br, &mut out, &lit, &dist)?;
            }
            _ => return Err("reserved block type".into()),
        }
        if bfinal == 1 {
            break;
        }
    }
    Ok((out, br.pos))
}

fn inflate_block(
    br: &mut BitReader,
    out: &mut Vec<u8>,
    lit: &Huffman,
    dist: &Huffman,
) -> Result<(), String> {
    loop {
        let sym = lit.decode(br)?;
        match sym {
            0..=255 => out.push(sym as u8),
            256 => return Ok(()),
            257..=285 => {
                let idx = sym as usize - 257;
                let len = LEN_BASE[idx] as usize + br.bits(u32::from(LEN_EXTRA[idx]))? as usize;
                let dsym = dist.decode(br)? as usize;
                if dsym >= 30 {
                    return Err("invalid distance symbol".into());
                }
                let d = DIST_BASE[dsym] as usize + br.bits(u32::from(DIST_EXTRA[dsym]))? as usize;
                if d > out.len() {
                    return Err("distance beyond start".into());
                }
                let start = out.len() - d;
                for k in 0..len {
                    let b = out[start + k];
                    out.push(b);
                }
            }
            _ => return Err("invalid literal/length symbol".into()),
        }
    }
}

/// zlib wrapper (RFC 1950): inflate the body and verify its adler32.
/// Trailing bytes after the adler (e.g. a newline before endstream) are
/// tolerated — the adler position comes from where inflate stopped.
pub fn zlib_decompress(data: &[u8]) -> Result<Vec<u8>, String> {
    if data.len() < 8 {
        return Err("zlib too short".into());
    }
    if data[0] & 0x0f != 8 {
        return Err("zlib: not deflate".into());
    }
    if (u16::from(data[0]) << 8 | u16::from(data[1])) % 31 != 0 {
        return Err("zlib header check failed".into());
    }
    let (out, consumed) = inflate_consumed(&data[2..])?;
    let aoff = 2 + consumed;
    if aoff + 4 > data.len() {
        return Err("zlib: missing adler32".into());
    }
    let given = u32::from_be_bytes([data[aoff], data[aoff + 1], data[aoff + 2], data[aoff + 3]]);
    if adler32(&out) != given {
        return Err("adler32 mismatch".into());
    }
    Ok(out)
}

pub fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in data.chunks(5552) {
        for &byte in chunk {
            a += u32::from(byte);
            b += a;
        }
        a %= 65_521;
        b %= 65_521;
    }
    (b << 16) | a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_roundtrip() {
        let raw = vec![1u8, 2, 0, 0xfd, 0xff, b'h', b'i'];
        assert_eq!(inflate(&raw).unwrap(), b"hi");
    }

    #[test]
    fn adler_reference() {
        assert_eq!(adler32(b"abc"), 0x024d_0127);
    }

    #[test]
    fn zlib_from_python_reference() {
        // zlib.compress(b"hello hello hello hello", 6)
        let z: Vec<u8> = vec![
            0x78, 0x9c, 0xcb, 0x48, 0xcd, 0xc9, 0xc9, 0x57, 0xc8, 0x40, 0x27, 0x01, 0x68, 0x03,
            0x08, 0xb1,
        ];
        // header + adler intact: decode via our zlib wrapper
        let out = zlib_decompress(&z).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(out, b"hello hello hello hello");
    }
}

#[cfg(test)]
mod z9 {
    use super::*;
    #[test]
    fn dynamic_block_real_stream() {
        let z: Vec<u8> = vec![
            0x78, 0x9c, 0x73, 0x0a, 0x51, 0xd0, 0x77, 0x33, 0x54, 0x30, 0x34, 0x51, 0x08, 0x49,
            0x53, 0x30, 0x37, 0x02, 0x22, 0x03, 0x85, 0x90, 0x14, 0x05, 0x0d, 0x8f, 0xd4, 0x9c,
            0x9c, 0x7c, 0x85, 0x00, 0x17, 0x37, 0x85, 0xf2, 0xfc, 0xa2, 0x9c, 0x14, 0x4d, 0x85,
            0x90, 0x2c, 0x05, 0xd7, 0x10, 0x2e, 0x27, 0xa8, 0x72, 0x43, 0x98, 0x72, 0x03, 0x88,
            0x72, 0xd7, 0xe2, 0xe4, 0xc4, 0x82, 0xd4, 0x62, 0x2b, 0x85, 0x18, 0x8d, 0x82, 0xc4,
            0xa2, 0xd4, 0xbc, 0x18, 0x4d, 0x85, 0x98, 0x18, 0x85, 0xa4, 0xc4, 0xe4, 0xec, 0xe2,
            0x9c, 0xc4, 0xe2, 0x0c, 0x85, 0x18, 0x43, 0x03, 0x43, 0x20, 0x36, 0x82, 0x99, 0x03,
            0x00, 0x34, 0x3f, 0x1f, 0xde,
        ];
        let out = zlib_decompress(&z).unwrap();
        let text = String::from_utf8_lossy(&out).into_owned();
        assert!(text.contains("Hello PDF world"), "{text}");
    }
}
