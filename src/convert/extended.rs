//! The extended data an application attaches to an entity: the dimension-style
//! variables a dimension or a leader sets for itself.

use uncad_model::model::{OverrideValue, StyleOverride};

use crate::entities::{self};
use crate::version::Version;

use super::entity::Context;
use super::hex;

/// The dimension-style variables a dimension sets for itself: the `DSTYLE`
/// list in its `ACAD` extended data, as (variable, value) pairs. An entity
/// without one sets none; data that does not read as the format describes it
/// ends the list there.
pub(super) fn style_overrides(entity: &entities::Entity, ctx: &Context<'_>) -> Vec<StyleOverride> {
    let unicode = ctx.version >= Version::R2007;
    let mut out = Vec::new();
    for data in &entity.common.extended_data {
        if Some(data.application) != ctx.acad {
            continue;
        }
        let items = extended_items(&data.bytes, unicode);
        // `1000 DSTYLE`, `1002 {`, pairs of a `1070` variable and a value, `1002 }`.
        let mut it = items.into_iter();
        if !matches!(it.next(), Some(Item::Text(t)) if t == "DSTYLE") {
            continue;
        }
        if !matches!(it.next(), Some(Item::Brace(false))) {
            continue;
        }
        while let Some(Item::Integer(variable)) = it.next() {
            let value = match it.next() {
                Some(Item::Real(v)) => OverrideValue::Real(v),
                Some(Item::Integer(v)) => OverrideValue::Integer(v),
                Some(Item::Text(t)) => OverrideValue::Text(t),
                Some(Item::Handle(h)) => OverrideValue::Handle(hex(h)),
                _ => break,
            };
            if let Ok(variable) = u16::try_from(variable) {
                out.push(StyleOverride { variable, value });
            }
        }
    }
    out
}

/// One item of an application's extended data.
pub(super) enum Item {
    Text(String),
    /// `1002`: `false` opens a group, `true` closes it.
    Brace(bool),
    Integer(i32),
    Real(f64),
    Handle(u64),
    /// Anything else the format allows (binary chunks, points, layer handles):
    /// it is skipped over, not read.
    Other,
}

/// The items of an application's extended data. Reading stops at the first
/// item the format does not describe.
pub(super) fn extended_items(bytes: &[u8], unicode: bool) -> Vec<Item> {
    let mut items = Vec::new();
    let mut i = 0usize;
    let take = |i: &mut usize, n: usize| -> Option<&[u8]> {
        let slice = bytes.get(*i..i.checked_add(n)?)?;
        *i += n;
        Some(slice)
    };
    while let Some(code) = bytes.get(i).copied() {
        i += 1;
        let item = match code {
            0 => {
                let text = if unicode {
                    let len =
                        take(&mut i, 2).map(|b| usize::from(u16::from_le_bytes([b[0], b[1]])));
                    len.and_then(|n| take(&mut i, n * 2)).and_then(|b| {
                        let units: Vec<u16> = (0..b.len() / 2)
                            .map(|k| u16::from_le_bytes([b[2 * k], b[2 * k + 1]]))
                            .collect();
                        String::from_utf16(&units).ok()
                    })
                } else {
                    let len = take(&mut i, 1).map(|b| usize::from(b[0]));
                    len.and_then(|n| take(&mut i, 2).and(take(&mut i, n)))
                        .and_then(|b| {
                            b.is_ascii()
                                .then(|| String::from_utf8_lossy(b).into_owned())
                        })
                };
                match text {
                    Some(t) => Item::Text(t),
                    None => break,
                }
            }
            2 => match take(&mut i, 1) {
                Some(b) => Item::Brace(b[0] != 0),
                None => break,
            },
            3 | 5 => match take(&mut i, 8) {
                Some(b) => {
                    let h = u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]);
                    if code == 5 {
                        Item::Handle(h)
                    } else {
                        Item::Other
                    }
                }
                None => break,
            },
            4 => match take(&mut i, 1)
                .map(|b| usize::from(b[0]))
                .and_then(|n| take(&mut i, n))
            {
                Some(_) => Item::Other,
                None => break,
            },
            10..=13 => match take(&mut i, 24) {
                Some(_) => Item::Other,
                None => break,
            },
            40..=42 => match take(&mut i, 8) {
                Some(b) => Item::Real(f64::from_le_bytes([
                    b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
                ])),
                None => break,
            },
            70 => match take(&mut i, 2) {
                Some(b) => Item::Integer(i32::from(i16::from_le_bytes([b[0], b[1]]))),
                None => break,
            },
            71 => match take(&mut i, 4) {
                Some(b) => Item::Integer(i32::from_le_bytes([b[0], b[1], b[2], b[3]])),
                None => break,
            },
            _ => break,
        };
        items.push(item);
    }
    items
}
