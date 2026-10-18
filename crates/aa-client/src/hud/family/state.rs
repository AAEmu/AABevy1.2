//! The native eight-row UI cache and actor-ID callbacks, separate from complete lane receipts.
use aa_net::family::{FamilyEvent, FamilyMember, FamilyRoster};
use std::collections::HashMap;

#[derive(Default)]
pub(in crate::hud) struct View {
    pub(super) character: u32,
    pub(super) object: u32,
    pub(super) family: u32,
    rosters: HashMap<u32, FamilyRoster>,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Notice {
    Added(Vec<u8>, Vec<u8>, Vec<u8>),
    Left(Vec<u8>, bool),
    Owner(Vec<u8>),
    Removed,
    Online(Vec<u8>, bool),
    Refresh,
}

pub(super) fn cstr(bytes: &[u8], maximum: usize) -> Vec<u8> {
    bytes.iter().take(maximum).copied().take_while(|byte| *byte != 0).collect()
}

fn owner(roster: &FamilyRoster) -> Vec<u8> {
    roster
        .members
        .iter()
        .rev()
        .find(|member| member.role != 0)
        .map_or_else(|| b"X".to_vec(), |member| member.name.clone())
}

impl View {
    pub(super) fn actor(&mut self, character: u32, object: u32, family: u32) {
        if self.character != character {
            *self = Self::default();
        }
        self.character = character;
        self.object = object;
        self.family = family;
    }

    pub(super) fn roster(&self) -> Option<&FamilyRoster> {
        (self.object != 0).then(|| self.rosters.get(&self.family)).flatten()
    }

    pub(super) fn is_owner(&self) -> bool {
        self.roster()
            .and_then(|roster| roster.members.iter().find(|member| member.character_id == self.character))
            .is_some_and(|member| member.role != 0)
    }

    pub(super) fn member(&self, name: &[u8]) -> Option<&FamilyMember> {
        let name = cstr(name, usize::MAX);
        self.roster()?.members.iter().find(|member| member.name == name)
    }

    fn callback(&mut self, character: u32, family: u32) {
        if self.object != 0 && self.character == character {
            self.family = family;
        }
    }

    fn replace(&mut self, roster: &FamilyRoster) {
        let mut roster = roster.clone();
        roster.members.truncate(8);
        for member in &mut roster.members {
            member.name = cstr(&member.name, 128);
            member.title = cstr(&member.title, 104);
            self.callback(member.character_id, roster.family_id);
        }
        // Replacing a roster only touches received rows; omitted actor IDs are not cleared.
        self.rosters.insert(roster.family_id, roster);
    }

    pub(super) fn receipt(&mut self, event: &FamilyEvent) -> Vec<Notice> {
        match event {
            FamilyEvent::Created(roster) => {
                self.replace(roster);
                let roster = &self.rosters[&roster.family_id];
                vec![Notice::Added(
                    roster.members.first().map_or_else(Vec::new, |member| member.name.clone()),
                    roster.members.get(1).map_or_else(Vec::new, |member| member.name.clone()),
                    roster.members.get(1).map_or_else(Vec::new, |member| member.title.clone()),
                )]
            }
            FamilyEvent::Described(roster) => {
                self.replace(roster);
                Vec::new()
            }
            FamilyEvent::MemberAdded { roster, added_index } => {
                self.replace(roster);
                let roster = &self.rosters[&roster.family_id];
                // Native directly indexes its inline row; reject invalid received indices as an app guard.
                let Some(member) = usize::try_from(*added_index).ok().and_then(|index| roster.members.get(index)) else {
                    return Vec::new();
                };
                vec![Notice::Added(owner(roster), member.name.clone(), member.title.clone())]
            }
            FamilyEvent::Removed { family_id } => {
                if let Some(roster) = self.rosters.remove(family_id) {
                    for member in roster.members {
                        self.callback(member.character_id, 0);
                    }
                }
                vec![Notice::Removed]
            }
            FamilyEvent::MemberRemoved {
                family_id,
                character_id,
                kicked,
            } => {
                let Some(roster) = self.rosters.get_mut(family_id) else { return Vec::new() };
                let Some(index) = roster.members.iter().position(|member| member.character_id == *character_id) else {
                    return Vec::new();
                };
                let removed = roster.members.remove(index);
                let dissolved = roster.members.len() < 2;
                self.callback(removed.character_id, 0);
                if dissolved {
                    for member in self.rosters.remove(family_id).unwrap().members {
                        self.callback(member.character_id, 0);
                    }
                }
                vec![Notice::Left(removed.name, *kicked)]
            }
            FamilyEvent::OwnerChanged { family_id, character_id } => {
                let Some(roster) = self.rosters.get_mut(family_id) else { return Vec::new() };
                let mut found = None;
                for member in &mut roster.members {
                    member.role = u8::from(member.character_id == *character_id);
                    if member.role != 0 {
                        member.title.clear();
                        found = Some(member.name.clone());
                    }
                }
                found.map_or_else(Vec::new, |name| vec![Notice::Owner(name)])
            }
            FamilyEvent::TitleChanged {
                family_id,
                character_id,
                title,
            } => {
                let Some(roster) = self.rosters.get_mut(family_id) else { return Vec::new() };
                let owner = owner(roster);
                let Some(member) = roster.members.iter_mut().find(|member| member.character_id == *character_id) else {
                    return Vec::new();
                };
                member.title = cstr(title, 104);
                vec![Notice::Added(owner, member.name.clone(), member.title.clone())]
            }
            FamilyEvent::NameChanged { family_id, character_id, name } => {
                let Some(member) = self
                    .rosters
                    .get_mut(family_id)
                    .and_then(|roster| roster.members.iter_mut().find(|member| member.character_id == *character_id))
                else {
                    return Vec::new();
                };
                member.name = cstr(name, 128);
                vec![Notice::Refresh]
            }
            FamilyEvent::MemberOnline {
                family_id,
                character_id,
                online,
            } => {
                let Some(member) = self
                    .rosters
                    .get_mut(family_id)
                    .and_then(|roster| roster.members.iter_mut().find(|member| member.character_id == *character_id))
                else {
                    return Vec::new();
                };
                member.online = *online;
                vec![Notice::Online(member.name.clone(), *online)]
            }
            FamilyEvent::Invitation(_) | FamilyEvent::Title(_) => Vec::new(),
        }
    }
}
