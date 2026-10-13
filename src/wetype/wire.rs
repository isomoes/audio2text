//! Minimal protobuf-wire helpers for the WeType protocol (no schema available).
use anyhow::{bail, Context, Result};

pub const MAX_MESSAGE: usize = 8 * 1024 * 1024;

pub fn varint(mut n: u64, out: &mut Vec<u8>) {
    while n > 127 {
        out.push((n as u8 & 127) | 128);
        n >>= 7;
    }
    out.push(n as u8);
}

pub fn number(out: &mut Vec<u8>, field: u32, value: u64) {
    varint(u64::from(field) * 8, out);
    varint(value, out);
}

pub fn bytes(out: &mut Vec<u8>, field: u32, value: impl AsRef<[u8]>) {
    let value = value.as_ref();
    varint(u64::from(field) * 8 + 2, out);
    varint(value.len() as u64, out);
    out.extend_from_slice(value);
}

#[derive(Debug, PartialEq)]
pub enum Value<'a> {
    Number(u64),
    Bytes(&'a [u8]),
}

fn read_varint(data: &[u8], offset: &mut usize) -> Result<u64> {
    let mut n = 0;
    for shift in (0..70).step_by(7) {
        let b = *data.get(*offset).context("Truncated protobuf varint")?;
        *offset += 1;
        if shift == 63 && b > 1 {
            bail!("Protobuf varint overflow");
        }
        n |= u64::from(b & 127) << shift;
        if b < 128 {
            return Ok(n);
        }
    }
    bail!("Invalid protobuf varint")
}

pub fn fields(data: &[u8]) -> Result<Vec<(u32, Value<'_>)>> {
    if data.len() > MAX_MESSAGE {
        bail!("Protobuf message too large");
    }
    let mut offset = 0;
    let mut out = Vec::new();
    while offset < data.len() {
        let tag = read_varint(data, &mut offset)?;
        if tag >> 3 == 0 || tag >> 3 > 0x1fffffff {
            bail!("Invalid protobuf field number");
        }
        let value = match tag & 7 {
            0 => Value::Number(read_varint(data, &mut offset)?),
            wire @ (1 | 2 | 5) => {
                let len = match wire {
                    1 => 8,
                    5 => 4,
                    _ => usize::try_from(read_varint(data, &mut offset)?)?,
                };
                let end = offset
                    .checked_add(len)
                    .context("Protobuf length overflow")?;
                let value = data.get(offset..end).context("Truncated protobuf field")?;
                offset = end;
                Value::Bytes(value)
            }
            _ => bail!("Unsupported protobuf wire type"),
        };
        out.push(((tag >> 3) as u32, value));
    }
    Ok(out)
}

pub fn get_bytes<'a>(fields: &[(u32, Value<'a>)], num: u32) -> Option<&'a [u8]> {
    fields.iter().rev().find_map(|(n, v)| match v {
        Value::Bytes(b) if *n == num => Some(*b),
        _ => None,
    })
}

pub fn get_number(fields: &[(u32, Value<'_>)], num: u32) -> Option<u64> {
    fields.iter().rev().find_map(|(n, v)| match v {
        Value::Number(v) if *n == num => Some(*v),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_roundtrip_and_last_value() {
        let mut data = Vec::new();
        number(&mut data, 1, u64::MAX);
        bytes(&mut data, 2, "中文");
        number(&mut data, 1, 42);
        let parsed = fields(&data).unwrap();
        assert_eq!(get_number(&parsed, 1), Some(42));
        assert_eq!(get_bytes(&parsed, 2), Some("中文".as_bytes()));
    }

    #[test]
    fn rejects_malformed_fields() {
        for data in [
            &[0][..],
            &[0x12, 0xff][..],
            &[0x12, 4, 1][..],
            &[0x0f][..],
            &[8, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 2][..],
        ] {
            assert!(fields(data).is_err());
        }
    }
}
