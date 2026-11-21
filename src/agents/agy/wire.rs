//! Just enough protobuf to read what agy keeps in its conversation database as blobs: the model
//! of each model call (`gen_metadata`). Everything else eye needs is in the plain JSON log.

/// One field of a message.
pub enum Field<'a> {
    Varint,
    Bytes(&'a [u8]),
}

fn varint(b: &[u8], i: &mut usize) -> Option<u64> {
    let (mut v, mut shift) = (0u64, 0u32);
    loop {
        let c = *b.get(*i)?;
        *i += 1;
        v |= u64::from(c & 0x7f).checked_shl(shift)?;
        if c & 0x80 == 0 {
            return Some(v);
        }
        shift += 7;
        if shift > 63 {
            return None;
        }
    }
}

/// The fields of a message in order, or None when the bytes are not a well-formed message.
pub fn fields(b: &[u8]) -> Option<Vec<(u32, Field<'_>)>> {
    let (mut i, mut out) = (0usize, vec![]);
    while i < b.len() {
        let key = varint(b, &mut i)?;
        let num = u32::try_from(key >> 3).ok()?;
        let f = match key & 7 {
            0 => {
                varint(b, &mut i)?;
                Field::Varint
            }
            1 => {
                i = i.checked_add(8).filter(|e| *e <= b.len())?;
                Field::Bytes(&b[i - 8..i])
            }
            5 => {
                i = i.checked_add(4).filter(|e| *e <= b.len())?;
                Field::Bytes(&b[i - 4..i])
            }
            2 => {
                let len = usize::try_from(varint(b, &mut i)?).ok()?;
                let end = i.checked_add(len).filter(|e| *e <= b.len())?;
                i = end;
                Field::Bytes(&b[end - len..end])
            }
            _ => return None,
        };
        out.push((num, f));
    }
    Some(out)
}

fn text(b: &[u8]) -> Option<String> {
    std::str::from_utf8(b)
        .ok()
        .filter(|s| !s.is_empty() && s.chars().all(|c| !c.is_control()))
        .map(String::from)
}

/// The model a `gen_metadata` row names: the display name (field 21 of the inner message), else
/// the model key (field 19).
pub fn model_name(row: &[u8]) -> Option<String> {
    let outer = fields(row)?;
    let inner = outer.iter().find_map(|(n, f)| match (n, f) {
        (1, Field::Bytes(b)) => Some(*b),
        _ => None,
    })?;
    let inner = fields(inner)?;
    let get = |want: u32| {
        inner.iter().find_map(|(n, f)| match f {
            Field::Bytes(b) if *n == want => text(b),
            _ => None,
        })
    };
    get(21).or_else(|| get(19))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(num: u32, b: &[u8]) -> Vec<u8> {
        let mut v = vec![(num << 3 | 2) as u8, b.len() as u8];
        v.extend_from_slice(b);
        v
    }

    #[test]
    fn the_model_is_read_from_the_inner_message() {
        let mut inner = bytes(19, b"flash-key");
        inner.extend(bytes(21, b"Test Flash (High)"));
        inner.extend([3 << 3, 0x96, 0x01]);
        let mut row = vec![2 << 3, 5];
        row.extend(bytes(1, &inner));
        assert_eq!(model_name(&row).as_deref(), Some("Test Flash (High)"));
        let only_key = bytes(1, &bytes(19, b"flash-key"));
        assert_eq!(model_name(&only_key).as_deref(), Some("flash-key"));
    }

    #[test]
    fn bad_bytes_are_none_not_a_panic() {
        for bad in [&[0x0a, 0x05, 1][..], &[0xff; 12][..], &[0x0b][..], &[][..]] {
            assert!(model_name(bad).is_none(), "{bad:?}");
        }
    }
}
