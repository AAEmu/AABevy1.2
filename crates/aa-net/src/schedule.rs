//! Schedule reward receipts. Timestamp and flag bytes stay in their received domains.
use crate::wire::{Reader, Result};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScheduleItem {
    pub schedule_type: u32,
    pub raw_gave: u8,
    pub accumulated: i32,
    pub raw_updated: i64,
}
impl ScheduleItem {
    pub fn read(r: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            schedule_type: r.u32()?,
            raw_gave: r.u8()?,
            accumulated: r.i32()?,
            raw_updated: r.i64()?,
        })
    }
}
/// Active bevy-lane framing consumes every declared row. Native397b0190 instead consumes
/// min(count,16); over16 is an explicit transport incompatibility, not native cache admission.
pub fn read_lane(r: &mut Reader<'_>) -> Result<(u8, Vec<ScheduleItem>)> {
    let declared_count = r.u8()?;
    let mut items = Vec::with_capacity(declared_count as usize);
    for _ in 0..declared_count {
        items.push(ScheduleItem::read(r)?);
    }
    Ok((declared_count, items))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_native_row_golden_retains_signed_timestamp_and_flag() {
        let row = [0x44, 0x33, 0x22, 0x11, 255, 0xfe, 255, 255, 255, 0, 0, 0, 0, 0, 0, 0, 128];
        let value = ScheduleItem::read(&mut Reader::new(&row)).unwrap();
        assert_eq!(
            value,
            ScheduleItem {
                schedule_type: 0x11223344,
                raw_gave: 255,
                accumulated: -2,
                raw_updated: i64::MIN
            }
        );
        for n in 0..row.len() {
            assert!(ScheduleItem::read(&mut Reader::new(&row[..n])).is_err());
        }
        assert_eq!(read_lane(&mut Reader::new(&[0])).unwrap(), (0, vec![]));
        let mut body = vec![32];
        for _ in 0..32 {
            body.extend_from_slice(&row);
        }
        let (count, items) = read_lane(&mut Reader::new(&body)).unwrap();
        assert_eq!(count, 32);
        assert_eq!(items, vec![value; 32]);
        for n in 0..body.len() {
            assert!(read_lane(&mut Reader::new(&body[..n])).is_err());
        }
    }
}
