//! Received family rosters. Lane framing preserves all rows; native UI displays at most eight.
use crate::{
    session::Event,
    wire::{Reader, Result, WireError},
};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FamilyMember {
    pub character_id: u32,
    pub name: Vec<u8>,
    pub role: u8,
    pub online: bool,
    pub title: Vec<u8>,
}
impl FamilyMember {
    pub fn name_text(&self) -> String {
        String::from_utf8_lossy(&self.name).trim_matches('\0').to_string()
    }
    pub fn title_text(&self) -> String {
        String::from_utf8_lossy(&self.title).trim_matches('\0').to_string()
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FamilyRoster {
    pub family_id: u32,
    pub members: Vec<FamilyMember>,
}
impl FamilyRoster {
    /// Active lane FamilyInfo::write; native398a85b0 agrees on fields but clamps its inline count8.
    pub fn read_lane(r: &mut Reader) -> Result<Self> {
        let family_id = r.u32()?;
        let count = r.i32()?;
        if count < 0 || count as usize > r.remaining() / 10 {
            return Err(WireError::Invalid("family member count exceeds payload".into()));
        }
        let mut members = Vec::with_capacity(count as usize);
        for _ in 0..count {
            members.push(FamilyMember {
                character_id: r.u32()?,
                name: r.blob()?.to_vec(),
                role: r.u8()?,
                online: r.bool()?,
                title: r.blob()?.to_vec(),
            });
        }
        Ok(Self { family_id, members })
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FamilyInvitation {
    pub inviter_id: u32,
    pub inviter_name: Vec<u8>,
    pub family_id: u32,
    pub role: Vec<u8>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FamilyTitle {
    pub unit_id: u32,
    pub role: u8,
    pub owner: Vec<u8>,
    pub title: Vec<u8>,
}
/// Every receipt is forwarded, including repeated values and notices that cannot fold into a roster.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FamilyEvent {
    Invitation(FamilyInvitation),
    Created(FamilyRoster),
    Described(FamilyRoster),
    MemberAdded { roster: FamilyRoster, added_index: i32 },
    Removed { family_id: u32 },
    MemberRemoved { family_id: u32, character_id: u32, kicked: bool },
    OwnerChanged { family_id: u32, character_id: u32 },
    TitleChanged { family_id: u32, character_id: u32, title: Vec<u8> },
    NameChanged { family_id: u32, character_id: u32, name: Vec<u8> },
    MemberOnline { family_id: u32, character_id: u32, online: bool },
    Title(FamilyTitle),
}
impl FamilyEvent {
    pub(crate) fn read_lane(opcode: u16, r: &mut Reader) -> Result<Self> {
        Ok(match opcode {
            0x02d => Self::Invitation(FamilyInvitation {
                inviter_id: r.u32()?,
                inviter_name: r.blob()?.to_vec(),
                family_id: r.u32()?,
                role: r.blob()?.to_vec(),
            }),
            0x02e => Self::Created(FamilyRoster::read_lane(r)?),
            0x02f => Self::Removed { family_id: r.u32()? },
            0x030 => Self::Described(FamilyRoster::read_lane(r)?),
            0x031 => Self::MemberAdded {
                roster: FamilyRoster::read_lane(r)?,
                added_index: r.i32()?,
            },
            0x032 => Self::MemberRemoved {
                family_id: r.u32()?,
                character_id: r.u32()?,
                kicked: r.bool()?,
            },
            0x033 => Self::OwnerChanged {
                family_id: r.u32()?,
                character_id: r.u32()?,
            },
            0x034 => Self::TitleChanged {
                family_id: r.u32()?,
                character_id: r.u32()?,
                title: r.blob()?.to_vec(),
            },
            0x035 => Self::NameChanged {
                family_id: r.u32()?,
                character_id: r.u32()?,
                name: r.blob()?.to_vec(),
            },
            0x036 => Self::Title(FamilyTitle {
                unit_id: r.bc()?,
                role: r.u8()?,
                owner: r.blob()?.to_vec(),
                title: r.blob()?.to_vec(),
            }),
            0x037 => Self::MemberOnline {
                family_id: r.u32()?,
                character_id: r.u32()?,
                online: r.bool()?,
            },
            _ => return Err(WireError::Invalid("not a family opcode".into())),
        })
    }
}
/// Authoritative map, matching native398a8df0's keyed roster cache. No own-family inference.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "bevy", derive(bevy::ecs::resource::Resource))]
pub struct Families {
    pub rosters: HashMap<u32, FamilyRoster>,
    pub last_invitation: Option<FamilyInvitation>,
    pub last_title: Option<FamilyTitle>,
}
impl Families {
    pub fn apply(&mut self, event: &Event) -> bool {
        match event {
            Event::Family(receipt) => self.apply_receipt(receipt),
            Event::CharacterSelected(_) | Event::Kicked { .. } | Event::LeftWorld { .. } | Event::Disconnected { .. } => {
                let changed = !self.rosters.is_empty() || self.last_invitation.is_some() || self.last_title.is_some();
                *self = Self::default();
                changed
            }
            _ => false,
        }
    }
    fn apply_receipt(&mut self, event: &FamilyEvent) -> bool {
        match event {
            FamilyEvent::Created(roster) | FamilyEvent::Described(roster) | FamilyEvent::MemberAdded { roster, .. } => {
                self.rosters.insert(roster.family_id, roster.clone()).as_ref() != Some(roster)
            }
            FamilyEvent::Removed { family_id } => self.rosters.remove(family_id).is_some(),
            FamilyEvent::Invitation(value) => {
                let changed = self.last_invitation.as_ref() != Some(value);
                self.last_invitation = Some(value.clone());
                changed
            }
            FamilyEvent::Title(value) => {
                let changed = self.last_title.as_ref() != Some(value);
                self.last_title = Some(value.clone());
                changed
            }
            FamilyEvent::MemberRemoved { family_id, character_id, .. } => {
                let Some(roster) = self.rosters.get_mut(family_id) else { return false };
                let Some(index) = roster.members.iter().position(|m| m.character_id == *character_id) else { return false };
                // Native398a8160 removes the first matching row, preserving the order of duplicates.
                roster.members.remove(index);
                if roster.members.len() < 2 {
                    self.rosters.remove(family_id);
                }
                true
            }
            FamilyEvent::OwnerChanged { family_id, character_id } => {
                let Some(roster) = self.rosters.get_mut(family_id) else { return false };
                let mut changed = false;
                for member in &mut roster.members {
                    let role = u8::from(member.character_id == *character_id);
                    changed |= member.role != role;
                    member.role = role;
                    // Native398a88ba/c0 also clears the new owner's family title.
                    if role == 1 {
                        changed |= !member.title.is_empty();
                        member.title.clear();
                    }
                }
                changed
            }
            FamilyEvent::TitleChanged {
                family_id,
                character_id,
                title,
            } => {
                let Some(member) = self.member_mut(*family_id, *character_id) else {
                    return false;
                };
                let changed = member.title != *title;
                member.title = title.clone();
                changed
            }
            FamilyEvent::NameChanged { family_id, character_id, name } => {
                let Some(member) = self.member_mut(*family_id, *character_id) else {
                    return false;
                };
                let changed = member.name != *name;
                member.name = name.clone();
                changed
            }
            FamilyEvent::MemberOnline {
                family_id,
                character_id,
                online,
            } => {
                let Some(member) = self.member_mut(*family_id, *character_id) else {
                    return false;
                };
                let changed = member.online != *online;
                member.online = *online;
                changed
            }
        }
    }
    fn member_mut(&mut self, family_id: u32, character_id: u32) -> Option<&mut FamilyMember> {
        self.rosters.get_mut(&family_id)?.members.iter_mut().find(|m| m.character_id == character_id)
    }
}
#[cfg(test)]
mod tests;
