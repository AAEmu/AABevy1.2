//! Party loot rolls. Loot keys are temporary corpse addresses, never inventory item IDs.
use crate::model::Item;
use crate::opcodes::{cs, sc};
use crate::session::Event;
use crate::state::LootFailure;
use crate::wire::{Reader, Result, WireError, Writer};
use std::collections::HashSet;
use std::time::{Duration, Instant};

pub const PROMPT_LIFETIME: Duration = Duration::from_secs(60);
pub const NEXT_PROMPT_DELAY: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LootRollKey {
    pub owner_type: u16,
    pub owner_obj_id: u32,
    pub item_index: u16,
}
impl LootRollKey {
    pub fn from_item_id(id: u64) -> Self {
        Self {
            item_index: id as u16,
            owner_type: (id >> 16) as u16,
            owner_obj_id: (id >> 32) as u32,
        }
    }
    pub fn from_item(item: &Item) -> Self {
        Self::from_item_id(item.id)
    }
    pub fn item_id(self) -> u64 {
        (u64::from(self.owner_obj_id) << 32)
            | (u64::from(self.owner_type) << 16)
            | u64::from(self.item_index)
    }
    pub fn validate(self) -> std::result::Result<(), &'static str> {
        if !matches!(self.owner_type, 1 | 2)
            || self.owner_obj_id == 0
            || self.owner_obj_id > 0xff_ffff
            || self.item_index == 0
        {
            Err("invalid loot roll owner or item index")
        } else {
            Ok(())
        }
    }
    fn read(r: &mut Reader) -> Result<Self> {
        let key = Self::from_item_id(r.u64()?);
        key.validate().map_err(|e| WireError::Invalid(e.into()))?;
        Ok(key)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LootRollCommand {
    pub key: LootRollKey,
    pub roll: bool,
}
impl LootRollCommand {
    pub fn validate(&self) -> std::result::Result<(), &'static str> {
        self.key.validate()
    }
    pub fn opcode(&self) -> u16 {
        cs::CSLootDicePacket
    }
    pub fn write(&self, w: &mut Writer) {
        w.u64(self.key.item_id()).bool(self.roll);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LootRollResult {
    pub character_id: u32,
    pub dice: i8,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LootRollSummary {
    pub key: LootRollKey,
    pub rolls: Vec<LootRollResult>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LootRollNotice {
    pub character_name: String,
    pub dice: i8,
}
#[derive(Clone, Debug, PartialEq)]
pub enum LootRollEvent {
    Offered(Item),
    Notified {
        character_name: String,
        item: Item,
        dice: i8,
    },
    Summary(LootRollSummary),
    Requested(LootRollCommand),
    RequestRejected {
        command: LootRollCommand,
        message: String,
    },
}
impl LootRollEvent {
    pub fn handles(op: u16) -> bool {
        (sc::SCLootDicePacket..=sc::SCLootDiceSummaryPacket).contains(&op)
    }
    pub fn name(&self) -> &'static str {
        match self {
            Self::Offered(_) => "SCLootDice",
            Self::Notified { .. } => "SCLootDiceNotify",
            Self::Summary(_) => "SCLootDiceSummary",
            Self::Requested(_) => "LootRollRequested",
            Self::RequestRejected { .. } => "LootRollRequestRejected",
        }
    }
    pub fn read(op: u16, r: &mut Reader) -> Result<Self> {
        fn item(r: &mut Reader) -> Result<Item> {
            let item =
                Item::read(r)?.ok_or_else(|| WireError::Invalid("empty loot roll item".into()))?;
            LootRollKey::from_item(&item)
                .validate()
                .map_err(|e| WireError::Invalid(e.into()))?;
            Ok(item)
        }
        Ok(match op {
            sc::SCLootDicePacket => Self::Offered(item(r)?),
            sc::SCLootDiceNotifyPacket => Self::Notified {
                character_name: r.string()?,
                item: item(r)?,
                dice: r.i8()?,
            },
            sc::SCLootDiceSummaryPacket => {
                let key = LootRollKey::read(r)?;
                let count = r.i32()?;
                if count < 0 || count as usize > r.remaining() / 5 {
                    return Err(WireError::Invalid(format!(
                        "invalid loot roll count {count}"
                    )));
                }
                let mut rolls = Vec::with_capacity(count as usize);
                let mut ids = HashSet::with_capacity(count as usize);
                for _ in 0..count {
                    let result = LootRollResult {
                        character_id: r.u32()?,
                        dice: r.i8()?,
                    };
                    if result.character_id == 0 || !ids.insert(result.character_id) {
                        return Err(WireError::Invalid(
                            "invalid/duplicate loot roller character ID".into(),
                        ));
                    }
                    rolls.push(result);
                }
                Self::Summary(LootRollSummary { key, rolls })
            }
            _ => {
                return Err(WireError::Invalid(format!(
                    "unknown loot roll opcode {op:03x}"
                )));
            }
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LootRollRound {
    pub key: LootRollKey,
    pub item: Option<Item>,
    /// Latest notification per name. These names are not guessed to match summary character IDs.
    pub notices: Vec<LootRollNotice>,
    pub summary: Option<Vec<LootRollResult>>,
    pub requested_choice: Option<bool>,
    pub pending: bool,
    pub own_roll: Option<i8>,
    /// SCLootItemTook is sent only to its actual recipient. Inventory tasks carry real item IDs.
    pub taken_by_me: bool,
    pub unavailable: bool,
    pub failure: Option<LootFailure>,
}
impl LootRollRound {
    fn new(key: LootRollKey) -> Self {
        Self {
            key,
            item: None,
            notices: vec![],
            summary: None,
            requested_choice: None,
            pending: false,
            own_roll: None,
            taken_by_me: false,
            unavailable: false,
            failure: None,
        }
    }
    pub fn finished(&self) -> bool {
        self.summary.is_some() || self.taken_by_me || self.unavailable
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LootRollPrompt {
    pub ui_key: u32,
    pub key: LootRollKey,
    pub received_at: Instant,
    pub shown: bool,
}
impl LootRollPrompt {
    pub fn age(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.received_at)
    }
    /// Translate the receipt's monotonic time to the app's X2Time:GetUiMsec clock origin.
    pub fn ui_timestamp_ms(&self, ui_now_ms: f64, now: Instant) -> f64 {
        ui_now_ms - self.age(now).as_secs_f64() * 1000.0
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LootRollFailure {
    pub command: LootRollCommand,
    pub message: String,
}
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "bevy", derive(bevy::ecs::resource::Resource))]
pub struct LootRolls {
    pub rounds: Vec<LootRollRound>,
    /// Active local UI queue, oldest first. Removing a prompt is not a server roll acknowledgement.
    pub prompts: Vec<LootRollPrompt>,
    pub max_concurrent: usize,
    pub last_summary: Option<LootRollSummary>,
    pub last_error: Option<LootRollFailure>,
    pub character_id: u32,
    pub character_name: String,
    next_ui_key: u32,
}
impl Default for LootRolls {
    fn default() -> Self {
        Self {
            rounds: vec![],
            prompts: vec![],
            max_concurrent: 3,
            last_summary: None,
            last_error: None,
            character_id: 0,
            character_name: String::new(),
            next_ui_key: 0,
        }
    }
}
impl LootRolls {
    pub fn round(&self, key: LootRollKey) -> Option<&LootRollRound> {
        self.rounds.iter().find(|r| r.key == key)
    }
    fn get(&mut self, key: LootRollKey) -> &mut LootRollRound {
        let index = self
            .rounds
            .iter()
            .position(|r| r.key == key)
            .unwrap_or_else(|| {
                self.rounds.push(LootRollRound::new(key));
                self.rounds.len() - 1
            });
        &mut self.rounds[index]
    }
    pub fn set_max_concurrent(&mut self, count: i32) {
        self.max_concurrent = count.max(1) as usize;
    }
    /// Same queue operation as native RequestNextDiceItem: prune >60s old local prompts, then
    /// expose one unshown row among the first max_concurrent rows. Never sends a pass or settles loot.
    pub fn next_prompt_at(&mut self, now: Instant) -> Option<LootRollPrompt> {
        self.prompts.retain(|p| p.age(now) <= PROMPT_LIFETIME);
        let end = self.max_concurrent.min(self.prompts.len());
        let prompt = self.prompts[..end].iter_mut().find(|p| !p.shown)?;
        prompt.shown = true;
        Some(prompt.clone())
    }
    pub fn next_prompt(&mut self) -> Option<LootRollPrompt> {
        self.next_prompt_at(Instant::now())
    }
    /// Native UI keys are opaque local integers; the 64-bit corpse item key is only on the wire.
    /// Expiry is not rejected here: the original Lua submits pass at its 60-second deadline.
    pub fn plan_action(
        &self,
        ui_key: u32,
        roll: bool,
    ) -> std::result::Result<LootRollCommand, &'static str> {
        let prompt = self
            .prompts
            .iter()
            .find(|p| p.ui_key == ui_key)
            .ok_or("unknown loot roll UI key")?;
        let round = self.round(prompt.key).ok_or("missing loot roll offer")?;
        if round.finished() || round.pending || round.own_roll.is_some() {
            return Err("loot roll is finished or already submitted");
        }
        let command = LootRollCommand {
            key: prompt.key,
            roll,
        };
        command.validate()?;
        Ok(command)
    }
    pub fn apply(&mut self, event: &Event) -> bool {
        self.apply_at(event, Instant::now())
    }
    pub fn apply_at(&mut self, event: &Event, now: Instant) -> bool {
        let Event::LootRoll(event) = event else {
            match event {
                Event::CharacterSelected(state) => {
                    *self = Self::default();
                    self.character_id = state.character.id;
                    self.character_name = state.character.name.clone();
                    return true;
                }
                Event::LeftWorld { .. } | Event::Disconnected { .. } => {
                    let changed = *self != Self::default();
                    *self = Self::default();
                    return changed;
                }
                Event::LootItemTook {
                    item_index,
                    owner_type,
                    owner_obj_id,
                    ..
                } => {
                    let key = LootRollKey {
                        item_index: *item_index,
                        owner_type: *owner_type,
                        owner_obj_id: *owner_obj_id,
                    };
                    let Some(round) = self.rounds.iter_mut().find(|r| r.key == key) else {
                        return false;
                    };
                    round.taken_by_me = true;
                    round.pending = false;
                    round.failure = None;
                    self.prompts.retain(|p| p.key != key);
                    return true;
                }
                Event::LootItemFailed {
                    error,
                    item_template_id,
                    item_index,
                    owner_type,
                    owner_obj_id,
                } => {
                    let key = LootRollKey {
                        item_index: *item_index,
                        owner_type: *owner_type,
                        owner_obj_id: *owner_obj_id,
                    };
                    let Some(round) = self.rounds.iter_mut().find(|r| r.key == key) else {
                        return false;
                    };
                    round.failure = Some(LootFailure {
                        error: *error,
                        item_template_id: *item_template_id,
                    });
                    return true;
                }
                _ => {}
            }
            let unavailable = |key: LootRollKey| match event {
                Event::UnitsRemoved(ids) => key.owner_type == 1 && ids.contains(&key.owner_obj_id),
                Event::DoodadsRemoved(ids) => {
                    key.owner_type == 2 && ids.contains(&key.owner_obj_id)
                }
                Event::Lootable {
                    owner_type,
                    obj_id,
                    has_loot: false,
                } => key.owner_type == *owner_type && key.owner_obj_id == *obj_id,
                _ => false,
            };
            let mut changed = false;
            for round in &mut self.rounds {
                if unavailable(round.key) {
                    round.unavailable = true;
                    round.pending = false;
                    changed = true;
                }
            }
            self.prompts.retain(|p| !unavailable(p.key));
            return changed;
        };
        match event {
            LootRollEvent::Offered(item) => {
                let key = LootRollKey::from_item(item);
                if self.round(key).is_some_and(LootRollRound::finished) {
                    *self.get(key) = LootRollRound::new(key);
                }
                self.get(key).item = Some(item.clone());
                if !self.prompts.iter().any(|p| p.key == key)
                    && !self
                        .round(key)
                        .is_some_and(|r| r.pending || r.own_roll.is_some())
                {
                    let ui_key = self.next_ui_key;
                    self.next_ui_key = self.next_ui_key.wrapping_add(1);
                    self.prompts.push(LootRollPrompt {
                        ui_key,
                        key,
                        received_at: now,
                        shown: false,
                    });
                }
            }
            LootRollEvent::Notified {
                character_name,
                item,
                dice,
            } => {
                let key = LootRollKey::from_item(item);
                let own = !self.character_name.is_empty()
                    && self.character_name.eq_ignore_ascii_case(character_name);
                let round = self.get(key);
                round.item = Some(item.clone());
                let notice = LootRollNotice {
                    character_name: character_name.clone(),
                    dice: *dice,
                };
                if let Some(old) = round
                    .notices
                    .iter_mut()
                    .find(|n| n.character_name.eq_ignore_ascii_case(character_name))
                {
                    *old = notice;
                } else {
                    round.notices.push(notice);
                }
                if own && *dice != 0 {
                    round.own_roll = Some(*dice);
                    round.pending = false;
                    self.prompts.retain(|p| p.key != key);
                }
            }
            LootRollEvent::Summary(summary) => {
                let own = summary
                    .rolls
                    .iter()
                    .find(|r| r.character_id == self.character_id && r.dice != 0)
                    .map(|r| r.dice);
                let round = self.get(summary.key);
                round.summary = Some(summary.rolls.clone());
                round.pending = false;
                if own.is_some() {
                    round.own_roll = own;
                }
                self.prompts.retain(|p| p.key != summary.key);
                self.last_summary = Some(summary.clone());
            }
            LootRollEvent::Requested(command) => {
                let round = self.get(command.key);
                round.requested_choice = Some(command.roll);
                round.pending = true;
                self.prompts.retain(|p| p.key != command.key);
                self.last_error = None;
            }
            LootRollEvent::RequestRejected { command, message } => {
                self.last_error = Some(LootRollFailure {
                    command: *command,
                    message: message.clone(),
                })
            }
        }
        true
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{ClientMsg, ServerMsg};
    use crate::items::{ItemTask, ItemTasks, action};
    use crate::model::{Character, CharacterState};
    use crate::state::WorldState;
    fn bytes(f: impl FnOnce(&mut Writer)) -> Vec<u8> {
        let mut w = Writer::new();
        f(&mut w);
        w.finish()
    }
    fn key(index: u16) -> LootRollKey {
        LootRollKey {
            owner_type: 1,
            owner_obj_id: 0xaabbcc,
            item_index: index,
        }
    }
    fn item_wire(key: LootRollKey, detail: u8) -> Vec<u8> {
        bytes(|w| {
            w.u32(23388)
                .u64(key.item_id())
                .u8(3)
                .u8(0)
                .i32(2)
                .u8(detail);
            match detail {
                1 => {
                    w.u32(0).u8(70).u16(8).u32(9).i64(10).u32(11);
                    for i in 0..7 {
                        w.u32(i + 12);
                    }
                    w.u16(20).u16(21);
                }
                4 => {
                    w.raw(&[1, 2, 3, 4, 5, 6, 7, 8, 9]);
                }
                _ => {}
            }
            w.i64(1700000000).i32(0).u32(0).u8(1).i64(0).i64(0);
        })
    }
    fn item(index: u16) -> Item {
        Item::read(&mut Reader::new(&item_wire(key(index), 1)))
            .unwrap()
            .unwrap()
    }
    fn apply(state: &mut LootRolls, event: LootRollEvent, now: Instant) {
        state.apply_at(&Event::LootRoll(event), now);
    }
    fn selected() -> Event {
        Event::CharacterSelected(Box::new(CharacterState {
            character: Character {
                id: 12,
                name: "Me".into(),
                ..Default::default()
            },
            ..Default::default()
        }))
    }
    fn batch(packets: &[(u16, Vec<u8>)]) -> Result<Vec<ServerMsg>> {
        let raw = bytes(|w| {
            for (op, body) in packets {
                w.u16(0).u16(*op).raw(body);
            }
            w.u16(0).u16(sc::SCOnOffSnowPacket).bool(true);
        });
        let len = raw.len() as u16;
        let wire = bytes(|w| {
            w.raw(&[0xdd, 4])
                .u16(packets.len() as u16 + 1)
                .u8(1)
                .u16(len)
                .u16(!len)
                .raw(&raw);
        });
        match ServerMsg::decode(&wire)? {
            ServerMsg::Batch(v) => Ok(v),
            _ => panic!(),
        }
    }
    #[test]
    fn native_request_is_exact_packed_corpse_item_id_and_bool_without_guessed_tail() {
        for roll in [false, true] {
            let command = LootRollCommand { key: key(2), roll };
            assert!(command.validate().is_ok());
            let msg = ClientMsg::LootRoll(command);
            assert_eq!(msg.opcode(), 0x91);
            assert_eq!(
                msg.encode_body(),
                [2, 0, 1, 0, 0xcc, 0xbb, 0xaa, 0, u8::from(roll)]
            );
        }
        assert_eq!(LootRollKey::from_item(&item(2)), key(2));
        for key in [
            LootRollKey {
                owner_type: 0,
                ..key(1)
            },
            LootRollKey {
                owner_type: 3,
                ..key(1)
            },
            LootRollKey {
                owner_obj_id: 0,
                ..key(1)
            },
            LootRollKey {
                owner_obj_id: 0x1000000,
                ..key(1)
            },
            key(0),
        ] {
            assert!(LootRollCommand { key, roll: true }.validate().is_err());
        }
    }
    #[test]
    fn all_roll_responses_preserve_dd04_boundary_through_the_following_loot_taken() {
        let packets = vec![
            (0xfb, item_wire(key(1), 1)),
            (
                0xfc,
                bytes(|w| {
                    w.string("Other").raw(&item_wire(key(1), 4)).i8(-1);
                }),
            ),
            (
                0xfd,
                bytes(|w| {
                    w.u64(key(1).item_id())
                        .i32(3)
                        .u32(12)
                        .i8(99)
                        .u32(13)
                        .i8(-1)
                        .u32(14)
                        .i8(0);
                }),
            ),
            (
                sc::SCLootItemTookPacket,
                bytes(|w| {
                    w.u32(23388).u64(key(1).item_id()).i32(2);
                }),
            ),
        ];
        let messages = batch(&packets).unwrap();
        assert_eq!(messages.len(), 5);
        assert_eq!(messages.last(), Some(&ServerMsg::Snow(true)));
        for (i, (op, raw)) in packets[..3].iter().enumerate() {
            let mut r = Reader::new(raw);
            let expected = LootRollEvent::read(*op, &mut r).unwrap();
            assert!(r.is_empty());
            assert_eq!(messages[i], ServerMsg::LootRoll(expected));
            for n in 0..raw.len() {
                assert!(
                    LootRollEvent::read(*op, &mut Reader::new(&raw[..n])).is_err(),
                    "op{op:03x} cut{n}"
                );
            }
        }
        let ServerMsg::LootRoll(LootRollEvent::Notified { item, dice, .. }) = &messages[1] else {
            panic!()
        };
        assert_eq!(*dice, -1);
        assert!(matches!(item.detail, crate::model::ItemDetail::Other(4)));
        assert_eq!(item.detail_payload.as_deref(), Some(&[1, 2, 3, 4, 5, 6, 7, 8, 9][..]));
        assert!(matches!(
            messages[3],
            ServerMsg::LootItemTook {
                item_index: 1,
                owner_type: 1,
                owner_obj_id: 0xaabbcc,
                ..
            }
        ));
    }
    #[test]
    fn count_bombs_invalid_keys_empty_items_and_duplicate_rollers_fail_strictly() {
        for count in [-1, 1, i32::MAX] {
            let raw = bytes(|w| {
                w.u64(key(1).item_id()).i32(count);
            });
            assert!(LootRollEvent::read(0xfd, &mut Reader::new(&raw)).is_err());
        }
        let raw = bytes(|w| {
            w.u64(key(1).item_id()).i32(2).u32(12).i8(1).u32(12).i8(2);
        });
        assert!(LootRollEvent::read(0xfd, &mut Reader::new(&raw)).is_err());
        // the body holds several zero pairs: the batch either resumes at the snow packet or, if a false packet
        // start also decodes to the end, stops rather than guessing
        assert!(matches!(batch(&[(0xfd, raw)]).unwrap().as_slice(),[crate::game::ServerMsg::DecodeFailed{..}] | [crate::game::ServerMsg::DecodeFailed{..}, crate::game::ServerMsg::Snow(true)]));
        assert!(LootRollEvent::read(0xfb, &mut Reader::new(&[0; 4])).is_err());
        assert!(LootRollEvent::read(0xfb, &mut Reader::new(&item_wire(key(0), 0))).is_err());
        let raw = bytes(|w| {
            w.u64(key(1).item_id()).i32(51);
            for id in 1..=51 {
                w.u32(id).i8(-1);
            }
        });
        // The lane can append outsiders: no fabricated 50-member wire cap is imposed here.
        assert!(LootRollEvent::read(0xfd, &mut Reader::new(&raw)).is_ok());
        let raw = bytes(|w| {
            w.u64(key(1).item_id()).i32(0);
        });
        assert!(LootRollEvent::read(0xfd, &mut Reader::new(&raw)).is_ok());
    }
    #[test]
    fn local_prompt_queue_has_three_slots_opaque_keys_and_receipt_based_ui_time() {
        let now = Instant::now();
        let mut state = LootRolls::default();
        state.apply_at(&selected(), now);
        for i in 1..=4 {
            apply(&mut state, LootRollEvent::Offered(item(i)), now);
        }
        let one = state.next_prompt_at(now).unwrap();
        let two = state.next_prompt_at(now).unwrap();
        let three = state.next_prompt_at(now).unwrap();
        assert_eq!((one.key, two.key, three.key), (key(1), key(2), key(3)));
        assert!(state.next_prompt_at(now).is_none());
        assert!(one.ui_key != two.ui_key && two.ui_key != three.ui_key);
        assert_ne!(u64::from(one.ui_key), one.key.item_id());
        assert_eq!(
            one.ui_timestamp_ms(7000.0, now + Duration::from_secs(2)),
            5000.0
        );
        let command = state.plan_action(one.ui_key, true).unwrap();
        apply(&mut state, LootRollEvent::Requested(command), now);
        let round = state.round(key(1)).unwrap();
        assert!(round.pending);
        assert_eq!(round.requested_choice, Some(true));
        assert!(round.own_roll.is_none());
        assert!(state.plan_action(one.ui_key, true).is_err());
        assert_eq!(state.next_prompt_at(now).unwrap().key, key(4));
        state.set_max_concurrent(0);
        assert_eq!(state.max_concurrent, 1);
        let original = state.prompts[0].clone();
        apply(
            &mut state,
            LootRollEvent::Offered(item(2)),
            now + Duration::from_secs(1),
        );
        assert_eq!(state.prompts.iter().filter(|p| p.key == key(2)).count(), 1);
        assert_eq!(state.prompts[0], original);
    }
    #[test]
    fn timeout_only_prunes_local_prompts_and_never_sends_pass_or_fakes_a_result() {
        let now = Instant::now();
        let mut state = LootRolls::default();
        apply(&mut state, LootRollEvent::Offered(item(1)), now);
        let prompt = state.next_prompt_at(now).unwrap();
        assert!(state.plan_action(prompt.ui_key, false).is_ok());
        state.next_prompt_at(now + PROMPT_LIFETIME);
        assert_eq!(state.prompts.len(), 1);
        state.next_prompt_at(now + PROMPT_LIFETIME + Duration::from_millis(1));
        assert!(state.prompts.is_empty());
        let round = state.round(key(1)).unwrap();
        assert!(
            !round.finished()
                && !round.pending
                && round.own_roll.is_none()
                && round.requested_choice.is_none()
        );
        assert!(state.plan_action(prompt.ui_key, false).is_err());
        assert!(state.last_summary.is_none());
    }
    #[test]
    fn notifications_summary_and_actual_take_are_distinct_from_inventory_settlement() {
        let now = Instant::now();
        let mut state = WorldState::default();
        state.inventory.bag = vec![None; 4];
        state.apply(&selected());
        state
            .loot_rolls
            .apply_at(&Event::LootRoll(LootRollEvent::Offered(item(1))), now);
        let prompt = state.loot_rolls.next_prompt_at(now).unwrap();
        let command = state.loot_rolls.plan_action(prompt.ui_key, true).unwrap();
        state.apply(&Event::LootRoll(LootRollEvent::Requested(command)));
        state.apply(&Event::LootRoll(LootRollEvent::Notified {
            character_name: "Other".into(),
            item: item(1),
            dice: 99,
        }));
        assert!(state.loot_rolls.round(key(1)).unwrap().pending);
        assert!(state.loot_rolls.round(key(1)).unwrap().own_roll.is_none());
        state.apply(&Event::LootRoll(LootRollEvent::Notified {
            character_name: "Me".into(),
            item: item(1),
            dice: 0,
        }));
        assert!(state.loot_rolls.round(key(1)).unwrap().pending);
        assert!(state.loot_rolls.round(key(1)).unwrap().own_roll.is_none());
        state.apply(&Event::LootRoll(LootRollEvent::Notified {
            character_name: "Me".into(),
            item: item(1),
            dice: 20,
        }));
        assert!(!state.loot_rolls.round(key(1)).unwrap().pending);
        assert_eq!(state.loot_rolls.round(key(1)).unwrap().own_roll, Some(20));
        state.apply(&Event::LootRoll(LootRollEvent::Summary(LootRollSummary {
            key: key(1),
            rolls: vec![
                LootRollResult {
                    character_id: 12,
                    dice: 20,
                },
                LootRollResult {
                    character_id: 13,
                    dice: 99,
                },
            ],
        })));
        assert!(state.loot_rolls.round(key(1)).unwrap().finished());
        assert!(!state.loot_rolls.round(key(1)).unwrap().taken_by_me);
        assert!(
            state.inventory.bag.iter().all(Option::is_none),
            "largest roll or summary cannot create an inventory item"
        );
        state.apply(&Event::LootItemTook {
            template_id: 23388,
            item_index: 1,
            owner_type: 1,
            owner_obj_id: 0xaabbcc,
            count: 2,
        });
        assert!(state.loot_rolls.round(key(1)).unwrap().taken_by_me);
        assert!(state.inventory.find_by_id(key(1).item_id()).is_none());
        let real = Item {
            id: 900,
            template_id: 23388,
            count: 2,
            grade: 3,
            ..Default::default()
        };
        state.apply(&Event::ItemsChanged(ItemTasks {
            reason: 9,
            tasks: vec![ItemTask::Add { raw_detail_block: None,
                action: action::CREATE,
                container: 2,
                slot: 0,
                item: real.clone(),
            }],
            ..Default::default()
        }));
        assert_eq!(
            state.inventory.find_by_id(900).map(|(_, _, i)| i),
            Some(&real)
        );
        assert!(state.inventory.find_by_id(key(1).item_id()).is_none());
    }
    #[test]
    fn all_pass_summary_allows_new_offer_and_owner_lifecycle_is_typed_and_authoritative() {
        let now = Instant::now();
        let mut state = LootRolls::default();
        state.apply_at(&selected(), now);
        apply(&mut state, LootRollEvent::Offered(item(1)), now);
        let old_ui = state.prompts[0].ui_key;
        apply(
            &mut state,
            LootRollEvent::Summary(LootRollSummary {
                key: key(1),
                rolls: vec![LootRollResult {
                    character_id: 12,
                    dice: -1,
                }],
            }),
            now,
        );
        assert!(state.prompts.is_empty());
        assert_eq!(state.round(key(1)).unwrap().own_roll, Some(-1));
        assert!(!state.round(key(1)).unwrap().taken_by_me);
        apply(&mut state, LootRollEvent::Offered(item(1)), now);
        assert_ne!(state.prompts[0].ui_key, old_ui);
        assert!(!state.round(key(1)).unwrap().finished());
        let mut doodad = item(1);
        doodad.id = LootRollKey {
            owner_type: 2,
            ..key(1)
        }
        .item_id();
        apply(&mut state, LootRollEvent::Offered(doodad), now);
        state.apply_at(&Event::UnitsRemoved(vec![0xaabbcc]), now);
        assert!(state.round(key(1)).unwrap().unavailable);
        assert_eq!(state.prompts.len(), 1);
        assert_eq!(state.prompts[0].key.owner_type, 2);
        state.apply_at(&Event::DoodadsRemoved(vec![0xaabbcc]), now);
        assert!(state.prompts.is_empty());
        state.apply_at(
            &Event::Disconnected {
                message: "test".into(),
            },
            now,
        );
        assert_eq!(state, LootRolls::default());
    }
    #[test]
    fn local_rejection_silent_server_error_and_bag_full_do_not_invent_roll_acceptance() {
        let now = Instant::now();
        let mut state = LootRolls::default();
        state.apply_at(&selected(), now);
        apply(&mut state, LootRollEvent::Offered(item(1)), now);
        let command = state.plan_action(state.prompts[0].ui_key, false).unwrap();
        apply(
            &mut state,
            LootRollEvent::RequestRejected {
                command,
                message: "test local rejection".into(),
            },
            now,
        );
        assert_eq!(state.prompts.len(), 1);
        assert!(!state.round(key(1)).unwrap().pending);
        assert!(state.last_error.is_some());
        apply(&mut state, LootRollEvent::Requested(command), now);
        state.apply_at(&Event::ServerError { code: 10 }, now);
        assert!(state.round(key(1)).unwrap().pending && state.last_error.is_none());
        state.apply_at(
            &Event::LootItemFailed {
                error: 25,
                item_template_id: 23388,
                item_index: 1,
                owner_type: 1,
                owner_obj_id: 0xaabbcc,
            },
            now,
        );
        let round = state.round(key(1)).unwrap();
        assert!(round.pending && round.own_roll.is_none() && !round.taken_by_me);
        assert_eq!(round.failure.as_ref().unwrap().error, 25);
        state.apply_at(
            &Event::Lootable {
                owner_type: 1,
                obj_id: 0xaabbcc,
                has_loot: false,
            },
            now,
        );
        assert!(state.round(key(1)).unwrap().unavailable && !state.round(key(1)).unwrap().pending);
        state.apply_at(&Event::LeftWorld { target: 0 }, now);
        assert_eq!(state, LootRolls::default());
    }
}
