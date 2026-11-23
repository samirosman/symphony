//! Bounded reader for the descriptor values used in Photoshop brush presets.
//! https://www.adobe.com/devnet-apps/photoshop/fileformatashtml/

use super::take;

pub(super) struct Object<'a>(Vec<(&'a [u8], Value<'a>)>);

enum Value<'a> {
    Object(Object<'a>),
    List(Vec<Value<'a>>),
    Text(String),
    Number(f32),
    Flag(bool),
    Ignored,
}

pub(super) fn presets(mut data: &[u8]) -> Option<Vec<Object<'_>>> {
    if data.len() > 16 * 1024 * 1024 || word(&mut data)? != 16 {
        return None;
    }
    let root = object(&mut data, 0)?;
    let (_, Value::List(list)) = root.0.into_iter().find(|(key, _)| *key == b"Brsh")? else {
        return None;
    };
    Some(
        list.into_iter()
            .filter_map(|v| match v {
                Value::Object(p) => Some(p),
                _ => None,
            })
            .collect(),
    )
}

impl<'a> Object<'a> {
    fn get(&self, key: &[u8]) -> Option<&Value<'a>> {
        self.0
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, value)| value)
    }

    pub(super) fn nested(&self, key: &[u8]) -> Option<&Self> {
        match self.get(key)? {
            Value::Object(o) => Some(o),
            _ => None,
        }
    }

    pub(super) fn text(&self, key: &[u8]) -> Option<&str> {
        match self.get(key)? {
            Value::Text(s) if !s.is_empty() => Some(s),
            _ => None,
        }
    }

    pub(super) fn number(&self, key: &[u8]) -> Option<f32> {
        match self.get(key)? {
            Value::Number(v) if v.is_finite() => Some(*v),
            _ => None,
        }
    }

    pub(super) fn flag(&self, key: &[u8]) -> bool {
        matches!(self.get(key), Some(Value::Flag(true)))
    }
}

fn word(data: &mut &[u8]) -> Option<u32> {
    Some(u32::from_be_bytes(take(data)?))
}

fn slice<'a>(data: &mut &'a [u8], len: usize) -> Option<&'a [u8]> {
    let (value, rest) = data.split_at_checked(len)?;
    *data = rest;
    Some(value)
}

fn string_bytes<'a>(data: &mut &'a [u8]) -> Option<&'a [u8]> {
    let size = (word(data)? as usize).checked_mul(2)?;
    slice(data, size)
}

fn id<'a>(data: &mut &'a [u8]) -> Option<&'a [u8]> {
    let len = word(data)? as usize;
    slice(data, if len == 0 { 4 } else { len })
}

fn object<'a>(data: &mut &'a [u8], depth: u8) -> Option<Object<'a>> {
    string_bytes(data)?;
    id(data)?;
    let count = word(data)?;
    if count > 10_000 {
        return None;
    }
    (0..count)
        .map(|_| Some((id(data)?, value(data, depth + 1)?)))
        .collect::<Option<Vec<_>>>()
        .map(Object)
}

fn value<'a>(data: &mut &'a [u8], depth: u8) -> Option<Value<'a>> {
    if depth > 16 {
        return None;
    }
    let ty = take::<4>(data)?;
    Some(match &ty {
        b"Objc" | b"GlbO" => Value::Object(object(data, depth)?),
        b"VlLs" => {
            let count = word(data)?;
            if count > 10_000 {
                return None;
            }
            Value::List(
                (0..count)
                    .map(|_| value(data, depth + 1))
                    .collect::<Option<Vec<_>>>()?,
            )
        }
        b"TEXT" => {
            let text = string_bytes(data)?;
            let units: Vec<_> = text
                .as_chunks::<2>()
                .0
                .iter()
                .take(4096)
                .copied()
                .map(u16::from_be_bytes)
                .collect();
            Value::Text(
                String::from_utf16_lossy(&units)
                    .trim_matches('\0')
                    .trim()
                    .to_owned(),
            )
        }
        b"UntF" => {
            take::<4>(data)?;
            Value::Number(f64::from_be_bytes(take(data)?) as f32)
        }
        b"doub" => Value::Number(f64::from_be_bytes(take(data)?) as f32),
        b"long" => Value::Number(i32::from_be_bytes(take(data)?) as f32),
        b"bool" => Value::Flag(take::<1>(data)?[0] != 0),
        b"enum" => {
            id(data)?;
            id(data)?;
            Value::Ignored
        }
        b"tdta" | b"alis" | b"Pth " => {
            let len = word(data)? as usize;
            slice(data, len)?;
            Value::Ignored
        }
        _ => return None,
    })
}
