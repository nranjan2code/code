//! Stream filters (ISO 32000-2 §7.4): the general-purpose decoders and the
//! PNG and TIFF predictors, each bounded to `limit` output bytes so a small
//! stream cannot inflate without limit. Image codecs (DCT, JPX, JBIG2,
//! CCITT) are never decoded: nothing here reads pixels.

use std::fmt;
use std::io::Read as _;

use crate::lexer::is_whitespace;
use crate::object::{Dict, Object};

/// Most filters one stream may chain.
const MAX_FILTERS: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FilterError {
    TooLarge,
    Unsupported(String),
    Corrupt(String),
}

impl fmt::Display for FilterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FilterError::TooLarge => f.write_str("a stream inflates past the size limit"),
            FilterError::Unsupported(name) => write!(f, "the {name} filter is not supported"),
            FilterError::Corrupt(message) => write!(f, "a stream is corrupt: {message}"),
        }
    }
}

/// Applies `filters` in order to `raw`.
pub(crate) fn decode(
    raw: &[u8],
    filters: &[(&[u8], Option<&Dict>)],
    limit: usize,
) -> Result<Vec<u8>, FilterError> {
    if filters.len() > MAX_FILTERS {
        return Err(FilterError::Corrupt("too many filters".into()));
    }
    let mut current: Option<Vec<u8>> = None;
    for (name, params) in filters {
        let input = current.as_deref().unwrap_or(raw);
        current = Some(one(name, input, *params, limit)?);
    }
    match current {
        Some(data) => Ok(data),
        None if raw.len() > limit => Err(FilterError::TooLarge),
        None => Ok(raw.to_vec()),
    }
}

fn one(
    name: &[u8],
    input: &[u8],
    params: Option<&Dict>,
    limit: usize,
) -> Result<Vec<u8>, FilterError> {
    match name {
        b"FlateDecode" | b"Fl" => predict(inflate(input, limit)?, params, limit),
        b"LZWDecode" | b"LZW" => {
            let early = params
                .and_then(|params| params.get(b"EarlyChange"))
                .and_then(Object::as_i64)
                .unwrap_or(1)
                != 0;
            predict(lzw(input, early, limit)?, params, limit)
        }
        b"ASCIIHexDecode" | b"AHx" => ascii_hex(input, limit),
        b"ASCII85Decode" | b"A85" => ascii85(input, limit),
        b"RunLengthDecode" | b"RL" => run_length(input, limit),
        other => Err(FilterError::Unsupported(
            String::from_utf8_lossy(other).into_owned(),
        )),
    }
}

/// Zlib data, or raw deflate data from a writer that left the header off.
/// A stream cut short or with a bad checksum keeps what inflated, as every
/// viewer does.
fn inflate(input: &[u8], limit: usize) -> Result<Vec<u8>, FilterError> {
    if input.iter().all(|byte| is_whitespace(*byte)) {
        return Ok(Vec::new());
    }
    let bound = limit as u64 + 1;
    let mut out = Vec::new();
    let zlib = flate2::read::ZlibDecoder::new(input)
        .take(bound)
        .read_to_end(&mut out);
    if out.len() > limit {
        return Err(FilterError::TooLarge);
    }
    if zlib.is_ok() || !out.is_empty() {
        return Ok(out);
    }
    let mut raw = Vec::new();
    let deflate = flate2::read::DeflateDecoder::new(input)
        .take(bound)
        .read_to_end(&mut raw);
    if raw.len() > limit {
        return Err(FilterError::TooLarge);
    }
    match deflate {
        Ok(_) => Ok(raw),
        Err(_) if !raw.is_empty() => Ok(raw),
        Err(error) => Err(FilterError::Corrupt(error.to_string())),
    }
}

fn lzw(input: &[u8], early_change: bool, limit: usize) -> Result<Vec<u8>, FilterError> {
    const CLEAR: usize = 256;
    const END: usize = 257;
    const NONE: u16 = u16::MAX;
    let mut prefix: Vec<u16> = Vec::with_capacity(4096);
    let mut suffix: Vec<u8> = Vec::with_capacity(4096);
    let reset = |prefix: &mut Vec<u16>, suffix: &mut Vec<u8>| {
        prefix.clear();
        suffix.clear();
        for byte in 0..=255u8 {
            prefix.push(NONE);
            suffix.push(byte);
        }
        prefix.extend([NONE, NONE]);
        suffix.extend([0, 0]);
    };
    reset(&mut prefix, &mut suffix);
    let sequence = |code: usize, prefix: &[u16], suffix: &[u8], out: &mut Vec<u8>| {
        out.clear();
        let mut current = code;
        while let (Some(&byte), Some(&next)) = (suffix.get(current), prefix.get(current)) {
            out.push(byte);
            if next == NONE || out.len() > 4096 {
                break;
            }
            current = usize::from(next);
        }
        out.reverse();
    };
    let mut out = Vec::new();
    let mut word = Vec::new();
    let mut previous: Option<usize> = None;
    let mut width = 9u32;
    let mut accumulator = 0u32;
    let mut bits = 0u32;
    for &byte in input {
        accumulator = (accumulator << 8) | u32::from(byte);
        bits += 8;
        while bits >= width {
            let code = ((accumulator >> (bits - width)) & ((1 << width) - 1)) as usize;
            bits -= width;
            accumulator &= (1 << bits) - 1;
            if code == CLEAR {
                reset(&mut prefix, &mut suffix);
                width = 9;
                previous = None;
                continue;
            }
            if code == END {
                return Ok(out);
            }
            let next_code = prefix.len();
            if code < next_code && code != CLEAR && code != END {
                sequence(code, &prefix, &suffix, &mut word);
                if let (Some(previous), Some(&first)) = (previous, word.first())
                    && next_code < 4096
                {
                    prefix.push(previous as u16);
                    suffix.push(first);
                }
            } else if code == next_code
                && let Some(previous) = previous
            {
                sequence(previous, &prefix, &suffix, &mut word);
                let Some(&first) = word.first() else {
                    return Err(FilterError::Corrupt("bad LZW code".into()));
                };
                word.push(first);
                if next_code < 4096 {
                    prefix.push(previous as u16);
                    suffix.push(first);
                }
            } else {
                return Err(FilterError::Corrupt("bad LZW code".into()));
            }
            out.extend_from_slice(&word);
            if out.len() > limit {
                return Err(FilterError::TooLarge);
            }
            previous = Some(code);
            let size = prefix.len() + usize::from(early_change);
            width = match size {
                0..512 => 9,
                512..1024 => 10,
                1024..2048 => 11,
                _ => 12,
            };
        }
    }
    Ok(out)
}

fn ascii_hex(input: &[u8], limit: usize) -> Result<Vec<u8>, FilterError> {
    let mut out = Vec::new();
    let mut high: Option<u8> = None;
    for &byte in input {
        let nibble = match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            b'A'..=b'F' => byte - b'A' + 10,
            b'>' => break,
            byte if is_whitespace(byte) => continue,
            _ => return Err(FilterError::Corrupt("bad ASCIIHex digit".into())),
        };
        match high.take() {
            Some(high) => out.push(high << 4 | nibble),
            None => high = Some(nibble),
        }
        if out.len() > limit {
            return Err(FilterError::TooLarge);
        }
    }
    if let Some(high) = high {
        out.push(high << 4);
    }
    Ok(out)
}

fn ascii85(input: &[u8], limit: usize) -> Result<Vec<u8>, FilterError> {
    let mut out = Vec::new();
    let mut group = [0u8; 5];
    let mut count = 0usize;
    let start = input
        .iter()
        .position(|byte| !is_whitespace(*byte))
        .unwrap_or(input.len());
    let body = input.get(start..).unwrap_or_default();
    let body = body.strip_prefix(b"<~").unwrap_or(body);
    for &byte in body {
        match byte {
            b'~' => break,
            b'z' if count == 0 => out.extend_from_slice(&[0; 4]),
            b'!'..=b'u' => {
                group[count] = byte - b'!';
                count += 1;
                if count == 5 {
                    let value = group
                        .iter()
                        .fold(0u64, |value, digit| value * 85 + u64::from(*digit));
                    let value = u32::try_from(value)
                        .map_err(|_| FilterError::Corrupt("bad ASCII85 group".into()))?;
                    out.extend_from_slice(&value.to_be_bytes());
                    count = 0;
                }
            }
            byte if is_whitespace(byte) => continue,
            _ => return Err(FilterError::Corrupt("bad ASCII85 digit".into())),
        }
        if out.len() > limit {
            return Err(FilterError::TooLarge);
        }
    }
    if count > 1 {
        for digit in group.iter_mut().skip(count) {
            *digit = 84;
        }
        let value = group
            .iter()
            .fold(0u64, |value, digit| value * 85 + u64::from(*digit));
        let bytes = u32::try_from(value)
            .map_err(|_| FilterError::Corrupt("bad ASCII85 group".into()))?
            .to_be_bytes();
        out.extend_from_slice(bytes.get(..count - 1).unwrap_or_default());
    }
    Ok(out)
}

fn run_length(input: &[u8], limit: usize) -> Result<Vec<u8>, FilterError> {
    let mut out = Vec::new();
    let mut index = 0usize;
    while let Some(&length) = input.get(index) {
        index += 1;
        match length {
            128 => break,
            0..=127 => {
                let count = usize::from(length) + 1;
                let Some(literal) = input.get(index..index + count) else {
                    out.extend_from_slice(input.get(index..).unwrap_or_default());
                    break;
                };
                out.extend_from_slice(literal);
                index += count;
            }
            _ => {
                let Some(&byte) = input.get(index) else {
                    break;
                };
                index += 1;
                out.extend(std::iter::repeat_n(byte, 257 - usize::from(length)));
            }
        }
        if out.len() > limit {
            return Err(FilterError::TooLarge);
        }
    }
    Ok(out)
}

fn predict(data: Vec<u8>, params: Option<&Dict>, limit: usize) -> Result<Vec<u8>, FilterError> {
    let Some(params) = params else {
        return Ok(data);
    };
    let int =
        |key: &[u8], default: i64| params.get(key).and_then(Object::as_i64).unwrap_or(default);
    let predictor = int(b"Predictor", 1);
    if predictor <= 1 {
        return Ok(data);
    }
    let colors = int(b"Colors", 1);
    let bits = int(b"BitsPerComponent", 8);
    let columns = int(b"Columns", 1);
    if !(1..=32).contains(&colors)
        || ![1, 2, 4, 8, 16].contains(&bits)
        || !(1..=1 << 24).contains(&columns)
    {
        return Err(FilterError::Corrupt("bad predictor parameters".into()));
    }
    let (colors, bits, columns) = (colors as usize, bits as usize, columns as usize);
    let pixel_bytes = (colors * bits).div_ceil(8).max(1);
    let row_bytes = (colors * bits)
        .checked_mul(columns)
        .map(|row_bits| row_bits.div_ceil(8))
        .ok_or_else(|| FilterError::Corrupt("bad predictor parameters".into()))?;
    if predictor == 2 {
        if bits != 8 {
            return Err(FilterError::Unsupported(
                "TIFF predictor below 8 bits".into(),
            ));
        }
        let mut data = data;
        for row in data.chunks_mut(row_bytes) {
            for index in pixel_bytes..row.len() {
                row[index] = row[index].wrapping_add(row[index - pixel_bytes]);
            }
        }
        return Ok(data);
    }
    let mut out = Vec::with_capacity(data.len());
    let mut previous = vec![0u8; row_bytes];
    for chunk in data.chunks(row_bytes + 1) {
        let Some((&kind, row)) = chunk.split_first() else {
            break;
        };
        let mut current = row.to_vec();
        for index in 0..current.len() {
            let left = if index >= pixel_bytes {
                current[index - pixel_bytes]
            } else {
                0
            };
            let up = previous.get(index).copied().unwrap_or(0);
            let up_left = if index >= pixel_bytes {
                previous.get(index - pixel_bytes).copied().unwrap_or(0)
            } else {
                0
            };
            current[index] = match kind {
                0 => current[index],
                1 => current[index].wrapping_add(left),
                2 => current[index].wrapping_add(up),
                3 => current[index].wrapping_add(((u16::from(left) + u16::from(up)) / 2) as u8),
                4 => current[index].wrapping_add(paeth(left, up, up_left)),
                _ => {
                    return Err(FilterError::Corrupt(format!(
                        "unknown PNG predictor {kind}"
                    )));
                }
            };
        }
        out.extend_from_slice(&current);
        if out.len() > limit {
            return Err(FilterError::TooLarge);
        }
        if let Some(target) = previous.get_mut(..current.len()) {
            target.copy_from_slice(&current);
        }
    }
    Ok(out)
}

fn paeth(left: u8, up: u8, up_left: u8) -> u8 {
    let estimate = i16::from(left) + i16::from(up) - i16::from(up_left);
    let to_left = (estimate - i16::from(left)).abs();
    let to_up = (estimate - i16::from(up)).abs();
    let to_up_left = (estimate - i16::from(up_left)).abs();
    if to_left <= to_up && to_left <= to_up_left {
        left
    } else if to_up <= to_up_left {
        up
    } else {
        up_left
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn ascii_filters_and_run_length() {
        assert_eq!(ascii_hex(b"48 65 6C6c 6F>", 100).unwrap(), b"Hello");
        assert_eq!(ascii85(b"<~87cURDZ~>", 100).unwrap(), b"Hello");
        assert_eq!(ascii85(b"z~>", 100).unwrap(), [0, 0, 0, 0]);
        assert_eq!(
            run_length(&[2, b'a', b'b', b'c', 254, b'x', 128], 100).unwrap(),
            b"abcxxx"
        );
    }

    #[test]
    fn lzw_decodes_the_specification_example() {
        let encoded = [0x80, 0x0B, 0x60, 0x50, 0x22, 0x0C, 0x0C, 0x85, 0x01];
        assert_eq!(lzw(&encoded, true, 100).unwrap(), b"-----A---B");
    }

    #[test]
    fn inflate_is_bounded_and_tolerates_a_missing_header() {
        use std::io::Write as _;
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&vec![0u8; 1 << 20]).unwrap();
        let bomb = encoder.finish().unwrap();
        assert_eq!(inflate(&bomb, 1024), Err(FilterError::TooLarge));
        assert_eq!(inflate(&bomb, 2 << 20).unwrap().len(), 1 << 20);

        let mut raw =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        raw.write_all(b"no header").unwrap();
        assert_eq!(inflate(&raw.finish().unwrap(), 100).unwrap(), b"no header");
    }

    #[test]
    fn png_up_predictor_rebuilds_rows() {
        let params = Dict(vec![
            (b"Predictor".to_vec(), Object::Int(12)),
            (b"Columns".to_vec(), Object::Int(3)),
        ]);
        let encoded = vec![2, 1, 2, 3, 2, 1, 1, 1];
        assert_eq!(
            predict(encoded, Some(&params), 100).unwrap(),
            [1, 2, 3, 2, 3, 4]
        );
    }

    #[test]
    fn image_codecs_are_never_decoded() {
        assert_eq!(
            decode(b"\xff\xd8", &[(b"DCTDecode", None)], 100),
            Err(FilterError::Unsupported("DCTDecode".into()))
        );
    }
}
