use std::mem;

use crate::consts::{
    DELTA_MAGIC, RS_OP_COPY_N1_N1, RS_OP_COPY_N8_N8, RS_OP_END, RS_OP_LITERAL_1, RS_OP_LITERAL_64,
    RS_OP_LITERAL_N1, RS_OP_LITERAL_N8,
};

/// Indicates that a delta could not be applied, either because it was invalid
/// or because a hash collision of some kind was detected.
#[derive(Debug)]
pub enum ApplyError {
    WrongMagic(u32),
    UnexpectedEof {
        reading: &'static str,
        expected: usize,
        available: usize,
    },
    OutputLimit(usize),
    CopyOutOfBounds,
    UnknownCommand(u8),
    TrailingData,
}

/// Apply `delta` to the base data `base`, writing the result to `out`.
/// Errors if more than `limit` bytes would be written to `out`.
pub fn apply_limited(
    base: &[u8],
    mut delta: &[u8],
    out: &mut Vec<u8>,
    mut limit: usize,
) -> Result<(), ApplyError> {
    macro_rules! read_n {
        ($n:expr, $what:expr) => {{
            let n = $n;
            if delta.len() < n {
                return Err(ApplyError::UnexpectedEof {
                    reading: $what,
                    expected: n,
                    available: delta.len(),
                });
            }
            let (prefix, rest) = delta.split_at(n);
            delta = rest;
            prefix
        }};
    }
    macro_rules! read_int {
        ($ty:ty, $what:expr) => {{
            let mut b = [0; mem::size_of::<$ty>()];
            b.copy_from_slice(read_n!(mem::size_of::<$ty>(), concat!("reading ", $what)));
            <$ty>::from_be_bytes(b)
        }};
    }
    macro_rules! read_varint {
        ($len:expr, $what:expr) => {{
            let len = $len;
            let mut b = [0; 8];
            b[8 - len..8].copy_from_slice(read_n!(len, concat!("reading ", $what)));
            u64::from_be_bytes(b)
        }};
    }
    macro_rules! safe_cast {
        ($val:expr, $ty:ty, $err:expr) => {{
            let val = $val;
            if val as u64 > <$ty>::max_value() as u64 {
                return Err($err);
            }
            val as $ty
        }};
    }
    macro_rules! safe_extend {
        ($slice:expr) => {{
            let slice: &[u8] = $slice;
            if slice.len() > limit {
                return Err(ApplyError::OutputLimit(slice.len() - limit));
            }
            limit -= slice.len();
            out.extend_from_slice(slice);
        }};
    }
    let magic = read_int!(u32, "magic");
    if magic != DELTA_MAGIC {
        return Err(ApplyError::WrongMagic(magic));
    }
    loop {
        let cmd = read_int!(u8, "cmd");
        match cmd {
            RS_OP_END => {
                break;
            }
            RS_OP_LITERAL_1..=RS_OP_LITERAL_N8 => {
                let n = if cmd <= RS_OP_LITERAL_64 {
                    // <=64, length is encoded in `cmd`
                    (1 + cmd - RS_OP_LITERAL_1) as usize
                } else {
                    safe_cast!(
                        read_varint!(1 << (cmd - RS_OP_LITERAL_N1) as usize, "literal length"),
                        usize,
                        ApplyError::OutputLimit(usize::max_value() - limit)
                    )
                };
                safe_extend!(read_n!(n, "literal"));
            }
            RS_OP_COPY_N1_N1..=RS_OP_COPY_N8_N8 => {
                let mode = cmd - RS_OP_COPY_N1_N1;
                let offset_len = 1 << (mode / 4) as usize;
                let len_len = 1 << (mode % 4) as usize;
                let offset = safe_cast!(
                    read_varint!(offset_len, "copy offset"),
                    usize,
                    ApplyError::CopyOutOfBounds
                );
                let len = safe_cast!(
                    read_varint!(len_len, "copy length"),
                    usize,
                    ApplyError::CopyOutOfBounds
                );
                if len == 0 {
                    return Err(ApplyError::CopyOutOfBounds);
                }
                let end = offset.checked_add(len).ok_or(ApplyError::CopyOutOfBounds)?;
                let subslice = base.get(offset..end).ok_or(ApplyError::CopyOutOfBounds)?;
                safe_extend!(subslice);
            }
            _ => return Err(ApplyError::UnknownCommand(cmd)),
        }
    }
    if delta.is_empty() {
        Ok(())
    } else {
        // extra content after EOF
        Err(ApplyError::TrailingData)
    }
}

/// Apply `delta` to the base data `base`, writing the result to `out`.
pub fn apply(base: &[u8], delta: &[u8], out: &mut Vec<u8>) -> Result<(), ApplyError> {
    apply_limited(base, delta, out, usize::max_value())
}
