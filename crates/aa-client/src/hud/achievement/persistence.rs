//! Type5 native Data hydration and leave-time saving. Unsupported payloads stay opaque.
use aa_net::{
    UiDataCommand, UiDataResponse,
    ui_data::{NativeData, NativeDataNode},
};

#[derive(Default)]
pub(super) struct State {
    pub owner: u32,
    tree: NativeData,
    save_allowed: bool,
}

#[derive(Default)]
pub(super) struct Roadmap {
    pub size: Option<i32>,
    pub visible: Option<bool>,
    pub npc_show: Option<i32>,
}

fn attribute<'a>(tree: &'a mut NativeData, key: &[u8]) -> &'a mut NativeDataNode {
    let index = tree.children.iter().position(|node| node.value == key).unwrap_or_else(|| {
        tree.children.push(NativeDataNode {
            value: key.to_vec(),
            children: Vec::new(),
        });
        tree.children.len() - 1
    });
    &mut tree.children[index]
}

fn leaf(value: Vec<u8>) -> NativeDataNode {
    NativeDataNode {
        value,
        children: Vec::new(),
    }
}

/// Actual3301bcb0/3301ac60 decimal conversion wraps; trailing non-digits are ignored.
fn atoi(bytes: &[u8]) -> u32 {
    let mut bytes = bytes;
    while matches!(bytes.first(), Some(b' ' | 9..=13)) {
        bytes = &bytes[1..];
    }
    let negative = bytes.first() == Some(&b'-');
    if matches!(bytes.first(), Some(b'+' | b'-')) {
        bytes = &bytes[1..];
    }
    let mut out = 0u32;
    for &byte in bytes.iter().take_while(|byte| byte.is_ascii_digit()) {
        out = out.wrapping_mul(10).wrapping_add(u32::from(byte - b'0'));
    }
    if negative { out.wrapping_neg() } else { out }
}

impl State {
    pub fn reset(&mut self, owner: u32) {
        *self = Self { owner, ..Self::default() };
    }

    pub fn record(&mut self, tracing: &[u32; 6]) {
        attribute(&mut self.tree, b"tracing_achievement").children =
            tracing.iter().map(|&id| leaf((id as i32).to_string().into_bytes())).collect();
    }

    pub fn roadmap(&self) -> Option<Roadmap> {
        let node = self.tree.children.iter().find(|node| node.value == b"roadmap_option")?;
        if node.children.is_empty() {
            return None;
        }
        let value = |key: &[u8]| node.children.iter().find(|n| n.value == key)?.children.first();
        let size = value(b"size").map(|n| atoi(&n.value) as i32);
        Some(Roadmap {
            size,
            visible: value(b"visible").map(|n| n.value == b"true"),
            //39443b5d reads a bool, but39443b6e writes the prior SIZE local as a
            //number. Missing size leaves native stack garbage: omit that case.
            npc_show: value(b"npcShow").and(size),
        })
    }

    pub fn record_roadmap(&mut self, size: Option<i32>, visible: Option<bool>, npc_show: Option<bool>) {
        let mut children = Vec::new();
        for (key, value) in [
            (b"size".as_slice(), size.map(|v| v.to_string())),
            (b"visible".as_slice(), visible.map(|v| v.to_string())),
            (b"npcShow".as_slice(), npc_show.map(|v| v.to_string())),
        ] {
            if let Some(value) = value {
                children.push(NativeDataNode { value: key.to_vec(), children: vec![leaf(value.into_bytes())] });
            }
        }
        //39443c44 clears only the first roadmap section; other type5 sections survive.
        attribute(&mut self.tree, b"roadmap_option").children = children;
    }

    fn slots(&self) -> [u32; 6] {
        let mut slots = [0; 6];
        if let Some(node) = self.tree.children.iter().find(|node| node.value == b"tracing_achievement") {
            for (slot, child) in slots.iter_mut().zip(&node.children) {
                *slot = atoi(&child.value);
            }
        }
        slots
    }

    pub fn receive(&mut self, response: &UiDataResponse) -> Option<[u32; 6]> {
        // Native ignores this packet identity. The app requires its actual active character.
        if response.data_type != 5 || self.owner == 0 || response.character_id != self.owner {
            return None;
        }
        if response.raw_length <= 0 {
            return Some(self.slots());
        }
        let bytes = response.payload.as_bytes();
        if response.raw_length as usize != bytes.len() && response.raw_length as usize != bytes.len() + 1 {
            self.save_allowed = false;
            bevy::log::warn!("achievement: refusing truncated or oversized type5 UI-data length");
            return None;
        }
        // The lane's missing-option empty/+1 response has no user subtrees. Treat it as an
        // explicit app empty state, avoiding the original parser's observed partial garbage node.
        let tree = if bytes.is_empty() {
            NativeData::default()
        } else {
            match NativeData::parse(bytes) {
                Ok(tree) => tree,
                Err(error) => {
                    self.save_allowed = false;
                    bevy::log::warn!("achievement: retaining unsupported type5 UI data: {error:?}");
                    return None;
                }
            }
        };
        self.tree = tree;
        self.save_allowed = true;
        Some(self.slots())
    }

    pub fn save(&mut self, character: u32) -> Option<UiDataCommand> {
        if !self.save_allowed || character == 0 || character != self.owner {
            return None;
        }
        //39217640 SetAttribute updates the FIRST exact-case version, or appends it at the end.
        attribute(&mut self.tree, b"version").children = vec![leaf(b"1".to_vec())];
        let bytes = match self.tree.encode() {
            Ok(bytes) => bytes,
            Err(error) => {
                bevy::log::warn!("achievement: cannot encode supported type5 tree: {error:?}");
                return None;
            }
        };
        let payload = String::from_utf8(bytes).ok()?;
        let command = UiDataCommand::Save {
            character_id: character,
            data_type: 5,
            payload,
        };
        command.validate().ok()?;
        Some(command)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn response(owner: u32, payload: &str, length: i32) -> UiDataResponse {
        UiDataResponse {
            character_id: owner,
            data_type: 5,
            payload: payload.into(),
            raw_length: length,
        }
    }
    #[test]
    fn actual_integer_wrapping_first_duplicates_sibling_preservation_and_native_save_text() {
        for (text, value) in [
            (" +42tail", 42),
            ("0x10", 0),
            ("2147483648", 0x80000000),
            ("4294967295", u32::MAX),
            ("4294967296", 0),
            ("-2147483649", 0x7fffffff),
            ("abc", 0),
        ] {
            assert_eq!(atoi(text.as_bytes()), value);
        }
        let text = "other ( a b, c d )\r\nversion 7\r\ntracing_achievement ( +42tail, -1, 0, 0, 0, 0, 99 )\r\nversion 9\r\ntracing_achievement 77\r\n";
        let mut state = State::default();
        state.reset(47);
        assert_eq!(
            state.receive(&response(47, text, text.len() as i32 + 1)),
            Some([42, u32::MAX, 0, 0, 0, 0])
        );
        state.record(&[2, u32::MAX, 0, 8, 0, 0]);
        let UiDataCommand::Save {
            character_id,
            data_type,
            payload,
        } = state.save(47).unwrap()
        else {
            panic!()
        };
        assert_eq!((character_id, data_type), (47, 5));
        assert_eq!(
            payload,
            "other ( a b, c d )\r\nversion 1\r\ntracing_achievement ( 2, -1, 0, 8, 0, 0 )\r\nversion 9\r\ntracing_achievement 77\r\n"
        );
        assert!(state.save(48).is_none());
    }
    #[test]
    fn opaque_unsupported_and_unsafe_lengths_never_overwrite_other_ui_data_or_owner() {
        let text = "tracing_achievement ( 2, 0, 0, 0, 0, 0 )\r\n";
        let mut state = State::default();
        state.reset(47);
        assert!(state.save(47).is_none());
        assert!(state.receive(&response(48, text, text.len() as i32)).is_none());
        assert_eq!(state.receive(&response(47, text, text.len() as i32)), Some([2, 0, 0, 0, 0, 0]));
        state.record(&[3, 0, 0, 0, 0, 0]);
        assert_eq!(state.receive(&response(47, "", 0)), Some([3, 0, 0, 0, 0, 0]));
        assert_eq!(state.receive(&response(47, "", -1)), Some([3, 0, 0, 0, 0, 0]));
        for bad in [
            response(47, text, 3),
            response(47, "# unknown metadata\r\n", 20),
            response(47, "x ( missing", 11),
        ] {
            assert!(state.receive(&bad).is_none());
            assert!(state.save(47).is_none());
            assert_eq!(state.slots(), [3, 0, 0, 0, 0, 0]);
        }
        assert_eq!(state.receive(&response(47, "", 1)), Some([0; 6]));
        state.record(&[9, 0, 0, 0, 0, 0]);
        let UiDataCommand::Save { payload, .. } = state.save(47).unwrap() else {
            panic!()
        };
        assert_eq!(payload, "tracing_achievement ( 9, 0, 0, 0, 0, 0 )\r\nversion 1\r\n");
        state.reset(48);
        assert!(state.save(47).is_none());
        assert!(state.save(48).is_none());
    }
}
