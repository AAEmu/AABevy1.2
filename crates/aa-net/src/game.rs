//! Game server messages.
//!
//! Every frame payload starts with `0xDD` and a level byte. Level 1 (`CS*` / `SC*`) continues with
//! `[hash][count][u16 opcode][body]`; level 2 is the CryNetwork proxy channel (`[u16 opcode][body]`);
//! level 4 is a compressed batch of server packets. The 1.2 protocol is neither encrypted nor
//! compressed otherwise, and the server verifies neither `hash` nor `count`.

use crate::buff::{Buff, BuffCaster};
use crate::items::{ActionSlot, InvenChunk, ItemMove, ItemTasks};
use crate::model::{
    ChatMessage, Character, CharacterState, CreateCharacter, Doodad, Item, Movement, Unit,
};
use crate::opcodes::{cs, pp, sc};
use crate::quest::{self, CompletedBlock, Quest};
use crate::skill::{PlotEvent, SkillFired, SkillStarted, SkillTarget, StartCaster, UnitDamage, UnitHeal};
use crate::unit_msg::{self, AggroEntry, Posture};
use crate::wire::{Pos, Reader, Result, WireError, Writer};

pub const MAGIC: u8 = 0xDD;
pub const LEVEL_GAME: u8 = 1;
pub const LEVEL_PROXY: u8 = 2;
pub const LEVEL_COMPRESSED: u8 = 4;
pub use crate::compression::MAX_INFLATED_BATCH_BYTES;

/// Where `CSLeaveWorld` leads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeaveTarget {
    /// Log out and close the game.
    Quit = 0,
    /// Back to the character list on the same world.
    CharacterSelect = 1,
    /// Back to the world list (needs a new login connection, see `SCReconnectAuth`).
    ServerSelect = 2,
}

impl LeaveTarget {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Quit),
            1 => Some(Self::CharacterSelect),
            2 => Some(Self::ServerSelect),
            _ => None,
        }
    }
}

/// Client side visual options sent with `CSSpawnCharacter`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VisualOptions {
    pub helmet: bool,
    pub back_holdable: bool,
    pub cosplay: bool,
    pub cosplay_backpack: bool,
}

impl Default for VisualOptions {
    fn default() -> Self {
        Self {
            helmet: true,
            back_holdable: true,
            cosplay: true,
            cosplay_backpack: true,
        }
    }
}

/// Messages the client sends to the game server.
#[derive(Clone, Debug, PartialEq)]
pub enum ClientMsg {
    /// `X2EnterWorldPacket`: opens the session with the login cookie.
    EnterWorld { account_id: u32, cookie: u32 },
    /// Proxy `FinishStatePacket`: acknowledges `ChangeState`.
    FinishState(i32),
    /// Proxy `PingPacket`; the server drops silent connections after 30 s.
    Ping { time_ms: i64 },
    ListCharacters,
    CreateCharacter(CreateCharacter),
    DeleteCharacter { id: u32 },
    CancelDeleteCharacter { id: u32 },
    SelectCharacter { id: u32 },
    SpawnCharacter(VisualOptions),
    NotifyInGame,
    NotifyInGameCompleted,
    Move { obj_id: u32, movement: Movement },
    /// Fixed CS089 type4 codec only; no Session driver/controller admission.
    ShipControl(crate::ShipControlRequest),
    Chat { chat_type: i16, target: String, message: String },
    ChangeTarget { obj_id: u32 },
    /// `CSSellBackpackGoods`: sell the equipped trade pack at the NPC `obj_id`.
    SellBackpackGoods { obj_id: u32 },
    /// `CSUpdateActionSlot`: puts `action` into hotbar slot `slot` (the original 1-based global slot number).
    UpdateActionSlot { slot: u8, action: ActionSlot },
    /// `CSSwapItems`: moves the item of one slot to another, swaps it with the item there, or merges two stacks of one
    /// kind; equipping and unequipping are swaps with the equipment container.
    SwapItems(ItemMove),
    /// `CSSplitBagItem`: moves `count` of a stack to another slot (a new stack, or onto a stack of the same kind).
    SplitItem { mv: ItemMove, count: i32 },
    /// `CSDestroyItem`: throws `count` of the item in `container` / `slot` away.
    DestroyItem { item_id: u64, container: u8, slot: u8, count: i32 },
    /// `CSResurrectCharacter`: stand up where the character died (`in_place`) or at the return
    /// point; the character must be dead.
    Resurrect { in_place: bool },
    /// Explicit Bloodlust state request; only SCForceAttackSet updates received state.
    SetForceAttack { on: bool },
    /// `CSStartSkill`: a unit, bag item or explicit mount caster starts `skill_id`.
    ItemEffect { self_obj_id:u32, request:crate::item_effect::ItemEffectRequest },
    /// CS049 conversion, or full item-to-item CS052 cleanup/Skinize.
    ItemLook { self_obj_id: u32, request: crate::look_conversion::ItemLookRequest },
    /// Native SaveNotes item cast with type5 composition grade; no staged upload.
    ComposeMusic { self_obj_id: u32, request: crate::MusicComposeRequest },
    StartSkill { skill_id: u32, self_obj_id: u32, caster: StartCaster, target: SkillTarget },
    /// `CSStopCasting`: the player cancels its cast or channel. `tl` is the timeline of a plain skill, `plot_tl` that of a plot
    /// (the real client sends 0 for the other one); the server checks `obj_id` is the player's unit.
    StopCasting { tl: u16, plot_tl: u16, obj_id: u32 },
    /// Exact native/lane CS055; explicit user cancellation uses reason0.
    RemoveBuff { obj_id: u32, buff_index: u32, reason: u8 },
    /// `CSExecuteCraft`: submit the whole recipe batch once. The session validates IDs/count.
    ExecuteCraft { craft_id: u32, doodad_obj_id: u32, count: i32 },
    /// `CSLootOpenBag`: open the loot of the corpse `obj_id` (`loot_all`: take everything at once; the server sends
    /// what is left as a `LootBag`).
    LootOpenBag { obj_id: u32, loot_all: bool },
    /// `CSLootItem`: take the item `item_index` out of the loot of `owner_obj_id` (`owner_type`: [`crate::state::owner_type`]).
    LootItem { item_index: u16, owner_type: u16, owner_obj_id: u32 },
    /// `CSLootCloseBag`: the loot window of `owner_obj_id` was closed.
    LootCloseBag { owner_type: u16, owner_obj_id: u32 },
    /// `CSLearnSkill`: spend skill points on a skill of one of the active abilities (answered by `SCSkillLearned`).
    LearnSkill { skill_id: u32 },
    /// `CSLearnBuff`: learn a passive buff, by its `passive_buffs` id (answered by `SCBuffLearned`).
    LearnBuff { passive_id: u32 },
    /// `CSResetSkills`: give back every skill point spent in `ability` (answered by `SCSkillsReset`).
    ResetSkills { ability: u8 },
    /// `CSSwapAbility`: replace the active ability `old` (11: an empty slot) by `ability` (answered by `SCAbilitySwapped`).
    SwapAbility { obj_id: u32, old: u8, ability: u8 },
    /// Talking to NPCs and the quest requests (`CSInteractNPC`, `CSStartQuestContext` ...), see [`crate::interaction`].
    Interact(crate::interaction::Interact),
    Shop(crate::shop::ShopCommand),
    /// Original CSICSMenuList (0x11b), with an empty body; session guards local identity.
    RequestInGameShopMenu,
    Bank(crate::bank::BankCommand),
    SetAutoUseAaPoint(bool),
    UnpackItem(crate::unpack::UnpackItemRequest),
    Expedition(crate::ExpeditionCommand),
    Dominion(crate::DominionCommand),
    Family(crate::FamilyCommand),
    Salon(crate::SalonCommand),
    /// Lobby requests ([`crate::account`]).
    Account(crate::AccountCommand),
    /// `/roll` and the player channel requests ([`crate::chat_command`]).
    ChatCommand(crate::ChatCommand),
    /// Native automatic response to a genuine server SummonJury; not a user invitation choice.
    JurySummoned { trial_id:u32,court:i32,jury_number:i32 },
    Trial(crate::trial::TrialCommand),
    Bounty(crate::BountyCommand),
    Ranking(crate::RankingCommand),
    RequestJuryWaitingNumber,
    /// `CSChallengeDuel`: the character id of the player to challenge.
    ChallengeDuel { challenged_id: u32 },
    /// `CSStartDuel`: answer the challenge of `challenger_id` (`error` 0 accepts, [`crate::duel::REJECTED`] refuses).
    StartDuel { challenger_id: u32, error: i16 },
    /// `CSExpressEmotion`: the player's emote (`express_texts` id), at `target_obj_id` (0: nobody). Answered by `SCEmotionExpressed`.
    ExpressEmotion { obj_id: u32, target_obj_id: u32, emotion_id: u32 },
    /// `CSChangeAppellation`: show the title `id` (0: none), see [`crate::appellation`].
    ChangeAppellation { id: u32 },
    LootRoll(crate::loot_roll::LootRollCommand),
    Battlefield(crate::battlefield::BattlefieldCommand),
    Actability(crate::actability::ActabilityCommand),
    Mate(crate::mate::MateCommand),
    Slave(crate::slave_control::SlaveCommand),
    /// Fixed Hang/Unhang codec only; no Session admission or predicted result.
    Hanging(crate::HangingRequest),
    UnbondDoodad(crate::UnbondDoodadRequest),
    Auction(crate::auction::AuctionCommand),
    Trade(crate::trade::TradeCommand),
    Mail(crate::mail::MailCommand),
    Friend(crate::friend::FriendCommand),
    Repair(crate::repair::RepairCommand),
    UiData(crate::ui_data::UiDataCommand),
    Music(crate::MusicCommand),
    /// Party command plus the local character name used by `CSReplyToJoinTeam`.
    Party { command: crate::party::PartyCommand, character_name: String },
    /// Answer to `SCTeleportUnit`; the server then accepts movement again.
    TeleportEnded { pos: Pos },
    /// The portal book and portals ([`crate::portal`]); `self_obj_id` casts the portal skills.
    Portal { self_obj_id: u32, command: crate::portal::PortalCommand },
    /// Housing requests (`CS*House*`), see [`crate::housing::HouseCommand`].
    House(crate::housing::HouseCommand),
    /// `CSCreateDoodad`: plant `item_id` as the doodad template `id`.
    CreateDoodad(crate::farm::CreateDoodad),
    /// `CSRequestCommonFarmList`.
    RequestCommonFarmList,
    /// Opening, closing and moving items of a coffer ([`crate::coffer`]).
    Coffer(crate::coffer::CofferCommand),
    LeaveWorld(LeaveTarget),
    CancelLeaveWorld,
}

impl ClientMsg {
    pub fn level(&self) -> u8 {
        match self {
            Self::FinishState(_) | Self::Ping { .. } => LEVEL_PROXY,
            _ => LEVEL_GAME,
        }
    }

    pub fn opcode(&self) -> u16 {
        match self {
            Self::EnterWorld { .. } => cs::X2EnterWorldPacket,
            Self::FinishState(_) => pp::FinishStatePacket,
            Self::Ping { .. } => pp::PingPacket,
            Self::ListCharacters => cs::CSListCharacterPacket,
            Self::CreateCharacter(_) => cs::CSCreateCharacterPacket,
            Self::DeleteCharacter { .. } => cs::CSDeleteCharacterPacket,
            Self::CancelDeleteCharacter { .. } => cs::CSCancelCharacterDeletePacket,
            Self::SelectCharacter { .. } => cs::CSSelectCharacterPacket,
            Self::SpawnCharacter(_) => cs::CSSpawnCharacterPacket,
            Self::NotifyInGame => cs::CSNotifyInGamePacket,
            Self::NotifyInGameCompleted => cs::CSNotifyInGameCompletedPacket,
            Self::Move { .. } | Self::ShipControl(_) => cs::CSMoveUnitPacket,
            Self::Chat { .. } => cs::CSSendChatMessagePacket,
            Self::ChangeTarget { .. } => cs::CSChangeTargetPacket,
            Self::SellBackpackGoods { .. } => cs::CSSellBackpackGoodsPacket,
            Self::UpdateActionSlot { .. } => cs::CSUpdateActionSlotPacket,
            Self::SwapItems(_) => cs::CSSwapItemsPacket,
            Self::SplitItem { .. } => cs::CSSplitBagItemPacket,
            Self::DestroyItem { .. } => cs::CSDestroyItemPacket,
            Self::Resurrect { .. } => cs::CSResurrectCharacterPacket,
            Self::SetForceAttack { .. } => cs::CSSetForceAttackPacket,
            Self::ItemEffect { .. } => cs::CSStartSkillPacket,
            Self::ItemLook { request, .. } => request.opcode(),
            Self::ComposeMusic { .. } => cs::CSStartSkillPacket,
            Self::StartSkill { .. } => cs::CSStartSkillPacket,
            Self::StopCasting { .. } => cs::CSStopCastingPacket,
            Self::RemoveBuff { .. } => cs::CSRemoveBuffPacket,
            Self::ExecuteCraft { .. } => cs::CSExecuteCraft,
            Self::LootOpenBag { .. } => cs::CSLootOpenBagPacket,
            Self::LootItem { .. } => cs::CSLootItemPacket,
            Self::LootCloseBag { .. } => cs::CSLootCloseBagPacket,
            Self::LearnSkill { .. } => cs::CSLearnSkillPacket,
            Self::LearnBuff { .. } => cs::CSLearnBuffPacket,
            Self::ResetSkills { .. } => cs::CSResetSkillsPacket,
            Self::SwapAbility { .. } => cs::CSSwapAbilityPacket,
            Self::Interact(m) => m.opcode(),
            Self::Shop(m) => m.opcode(),
            Self::RequestInGameShopMenu => cs::CSICSMenuListPacket,
            Self::Bank(m) => m.opcode(),
            Self::SetAutoUseAaPoint(_) => cs::CSChangeAutoUseAAPointPacket,
            Self::UnpackItem(_) => cs::CSThisTimeUnpackItemPacket,
            Self::Expedition(command) => command.opcode(),
            Self::Dominion(command) => command.opcode(),
            Self::Family(command) => command.opcode(),
            Self::Salon(command) => command.opcode(),
            Self::Account(command) => command.opcode(),
            Self::ChatCommand(command) => command.opcode(),
            Self::JurySummoned{..} => cs::CSJurySummonedPacket,
            Self::Trial(command) => command.opcode(),
            Self::Bounty(command) => command.opcode(),
            Self::Ranking(command) => command.opcode(),
            Self::RequestJuryWaitingNumber => cs::CSRequestJuryWaitingNumberPacket,
            Self::ChallengeDuel { .. } => cs::CSChallengeDuelPacket,
            Self::StartDuel { .. } => cs::CSStartDuelPacket,
            Self::ExpressEmotion { .. } => cs::CSExpressEmotionPacket,
            Self::ChangeAppellation { .. } => cs::CSChangeAppellationPacket,
            Self::LootRoll(m) => m.opcode(),
            Self::Battlefield(m) => m.opcode(),
            Self::Actability(m) => m.opcode(),
            Self::Mate(m) => m.opcode(),
            Self::Slave(m) => m.opcode(),
            Self::Hanging(request) => request.opcode(),
            Self::UnbondDoodad(_) => cs::CSUnbondDoodadPacket,
            Self::Auction(m) => m.opcode(),
            Self::Trade(m) => m.opcode(),
            Self::Mail(m) => m.opcode(),
            Self::Friend(m) => m.opcode(),
            Self::Repair(m) => m.opcode(),
            Self::UiData(m) => m.opcode(),
            Self::Music(m) => m.opcode(),
            Self::Party { command, .. } => command.opcode(),
            Self::TeleportEnded { .. } => cs::CSTeleportEndedPacket,
            Self::LeaveWorld(_) => cs::CSLeaveWorldPacket,
            Self::CancelLeaveWorld => cs::CSCancelLeaveWorldPacket,
            Self::Portal { command, .. } => command.opcode(),
            Self::House(command) => command.opcode(),
            Self::CreateDoodad(_) => cs::CSCreateDoodadPacket,
            Self::RequestCommonFarmList => cs::CSRequestCommonFarmList,
            Self::Coffer(command) => command.opcode(),
        }
    }

    pub fn name(&self) -> &'static str {
        if let Self::Dominion(command) = self {
            return match command {
                crate::DominionCommand::ChangeTaxRate { .. } => "CSUpdateDominionTaxRatePacket",
                crate::DominionCommand::ChangeNationalTaxRate { .. } => "CSUpdateNationalTaxRatePacket",
                crate::DominionCommand::RequestCharacterBrief { .. } => "CSRequestCharBriefPacket",
            };
        }
        if self.level() == LEVEL_PROXY {
            pp::name(self.opcode())
        } else {
            cs::name(self.opcode())
        }
    }

    pub fn encode_body(&self) -> Vec<u8> {
        let mut w = Writer::new();
        match self {
            Self::EnterWorld { account_id, cookie } => {
                w.u32(0) // pFrom
                    .u32(0) // pTo
                    .u32(*account_id)
                    .u32(*cookie)
                    .i32(0) // zoneId
                    .u8(0) // tb
                    .u64(0); // revision
            }
            Self::FinishState(state) => {
                w.i32(*state);
            }
            Self::Ping { time_ms } => {
                w.i64(*time_ms).i64(*time_ms).u32(*time_ms as u32);
            }
            Self::ListCharacters => {
                w.i32(0).blob(&[]);
            }
            Self::CreateCharacter(c) => c.write(&mut w),
            Self::DeleteCharacter { id } | Self::CancelDeleteCharacter { id } => {
                w.u32(*id);
            }
            Self::ChangeTarget { obj_id } | Self::SellBackpackGoods { obj_id } => {
                w.bc(*obj_id);
            }
            Self::UpdateActionSlot { slot, action } => {
                w.u8(*slot);
                action.write(&mut w);
            }
            Self::SwapItems(mv) => mv.write(&mut w),
            Self::SplitItem { mv, count } => {
                mv.write(&mut w);
                w.i32(*count);
            }
            Self::DestroyItem { item_id, container, slot, count } => {
                w.u64(*item_id).u8(*container).u8(*slot).i32(*count);
            }
            Self::SetForceAttack { on } => { w.bool(*on); }
            Self::Resurrect { in_place } => {
                w.bool(*in_place);
            }
            Self::ItemEffect { self_obj_id,request } => request.write(&mut w,*self_obj_id),
            Self::ItemLook { self_obj_id, request } => request.write(&mut w, *self_obj_id),
            Self::ComposeMusic { self_obj_id, request } => request.write(&mut w, *self_obj_id),
            Self::StartSkill { skill_id, self_obj_id, caster, target } => {
                crate::skill::write_start_skill(&mut w, *skill_id, *self_obj_id, *caster, target);
            }
            Self::StopCasting { tl, plot_tl, obj_id } => {
                w.u16(*tl).u16(*plot_tl).bc(*obj_id);
            }
            Self::RemoveBuff { obj_id, buff_index, reason } => {
                w.bc(*obj_id).u32(*buff_index).u8(*reason);
            }
            Self::ExecuteCraft { craft_id, doodad_obj_id, count } => {
                w.u32(*craft_id).bc(*doodad_obj_id).i32(*count);
            }
            Self::LootOpenBag { obj_id, loot_all } => {
                // the object, the second object (none) and whether to take everything
                w.bc(*obj_id).bc(0).bool(*loot_all);
            }
            Self::LootItem { item_index, owner_type, owner_obj_id } => {
                w.u16(*item_index).u16(*owner_type).bc(*owner_obj_id).u16(0).u16(0);
            }
            Self::LootCloseBag { owner_type, owner_obj_id } => {
                w.u16(0).u16(*owner_type).bc(*owner_obj_id).u8(0);
            }
            Self::LearnSkill { skill_id } => {
                w.u32(*skill_id);
            }
            Self::LearnBuff { passive_id } => {
                w.u32(*passive_id);
            }
            Self::ResetSkills { ability } => {
                w.u8(*ability).bool(false); // ausp
            }
            Self::SwapAbility { obj_id, old, ability } => {
                w.bc(*obj_id).u8(*old).u8(*ability).bool(false); // auap
            }
            Self::Interact(m) => m.write(&mut w),
            Self::Shop(m) => m.write(&mut w),
            Self::RequestInGameShopMenu => {},
            Self::Bank(m) => m.write(&mut w),
            Self::SetAutoUseAaPoint(value) => { w.bool(*value); },
            Self::UnpackItem(request) => request.write(&mut w),
            Self::Expedition(command) => command.write(&mut w),
            Self::Dominion(command) => command.write(&mut w),
            Self::Family(command) => command.write(&mut w),
            Self::Salon(command) => command.write(&mut w),
            Self::Account(command) => command.write(&mut w),
            Self::ChatCommand(command) => command.write(&mut w),
            Self::JurySummoned{trial_id,court,jury_number} => {w.u32(*trial_id).i32(*court).i32(*jury_number);},
            Self::Trial(command) => command.write(&mut w),
            Self::Bounty(command) => command.write(&mut w),
            Self::Ranking(command) => command.write(&mut w),
            Self::RequestJuryWaitingNumber => {},
            Self::ChallengeDuel { challenged_id } => { w.u32(*challenged_id); }
            Self::StartDuel { challenger_id, error } => { w.u32(*challenger_id).i16(*error); }
            Self::ExpressEmotion { obj_id, target_obj_id, emotion_id } => { w.bc(*obj_id).bc(*target_obj_id).u32(*emotion_id); }
            Self::ChangeAppellation { id } => { w.u32(*id); }
            Self::LootRoll(m) => m.write(&mut w),
            Self::Battlefield(m) => m.write(&mut w),
            Self::Actability(m) => m.write(&mut w),
            Self::Mate(m) => m.write(&mut w),
            Self::Slave(m) => m.write(&mut w),
            Self::Hanging(request) => request.write(&mut w),
            Self::ShipControl(request) => request.write(&mut w),
            Self::UnbondDoodad(request) => { w.bc(request.actor_obj_id).bc(request.bond.owner_obj_id); }
            Self::Auction(m) => m.write(&mut w),
            Self::Trade(m) => m.write(&mut w),
            Self::Mail(m) => m.write(&mut w),
            Self::Friend(m) => m.write(&mut w),
            Self::Repair(m) => m.write(&mut w),
            Self::Portal { self_obj_id, command } => command.write(&mut w, *self_obj_id),
            Self::UiData(m) => m.write(&mut w),
            Self::Music(m) => m.write(&mut w),
            Self::Party { command, character_name } => command.write(&mut w, character_name),
            Self::SelectCharacter { id } => {
                w.u32(*id).bool(false).u8(0);
            }
            Self::SpawnCharacter(v) => {
                // All five option groups present: stp (6 bytes), helmet, back holdable, cosplay,
                // cosplay backpack.
                w.u8(0x1F)
                    .raw(&[0u8; 6])
                    .bool(v.helmet)
                    .bool(v.back_holdable)
                    .bool(v.cosplay)
                    .bool(v.cosplay_backpack);
            }
            Self::NotifyInGame | Self::NotifyInGameCompleted | Self::CancelLeaveWorld => {}
            Self::Move { obj_id, movement } => {
                w.bc(*obj_id);
                movement.write(&mut w);
            }
            Self::Chat { chat_type, target, message } => {
                w.i16(*chat_type)
                    .i16(0)
                    .i32(0)
                    .string(target)
                    .string(message)
                    .u8(0) // language
                    .i32(0); // ability
            }
            Self::TeleportEnded { pos } => {
                w.i64(crate::wire::coord_to_long(pos.x))
                    .i64(crate::wire::coord_to_long(pos.y))
                    .f32(pos.z)
                    .raw(&[0u8; 12])
                    .f32(1.0); // identity rotation
            }
            Self::LeaveWorld(target) => {
                w.u8(*target as u8);
            }
            Self::House(command) => command.write(&mut w),
            Self::CreateDoodad(request) => request.write(&mut w),
            Self::RequestCommonFarmList => {}
            Self::Coffer(command) => command.write(&mut w),
        }
        w.finish()
    }

    /// The frame payload (without the `u16` length). `counter` is the level-1 packet counter.
    pub fn encode(&self, counter: u8) -> Vec<u8> {
        let mut out = vec![MAGIC, self.level()];
        if self.level() == LEVEL_GAME {
            out.extend([0, counter]);
        }
        out.extend(self.opcode().to_le_bytes());
        out.extend(self.encode_body());
        out
    }
}

/// Messages the game server sends that this client understands.
#[derive(Clone, Debug, PartialEq)]
pub enum ServerMsg {
    // -- proxy channel
    /// CryNetwork state machine: answer with `FinishState` of the same state.
    ChangeState(i32),
    Pong(crate::proxy::ProxyPong),
    // -- lobby
    EnterWorldResponse { reason: i16, gm: bool, stream_token: u32, stream_port: u16, world_time: i64 },
    /// The server asks the client to reconnect to the login server (return to world select).
    ReconnectAuth { token: u32 },
    SlotCount(u8),
    AccountInfo { pay_method: i32, pay_location: i32, pay_start: i64, pay_end: i64, real_pay_time: i64 },
    PremiumPoint { point: i32, old_grade: u8, grade: u8 },
    /// Congestion per race (index = race id) for character creation.
    RaceCongestion([u8; 9]),
    CharacterList { last: bool, characters: Vec<Character> },
    UiData(crate::ui_data::UiDataResponse),
    CharacterCreated(Box<Character>),
    CharacterCreationFailed { reason: u8 },
    CharacterDeleteResponse { id: u32, status: u8, delete_request_time: i64, delete_time: i64 },
    CancelCharacterDeleteResponse { id: u32, status: u8 },
    CharacterDeleted { id: u32, name: String },
    // -- entering the world
    CharacterState(Box<CharacterState>),
    // -- world
    UnitState(Box<Unit>),
    UnitsRemoved(Vec<u32>),
    UnitMoved { obj_id: u32, movement: Movement },
    /// `SCUnitMovements`: several units moved at once (read as unit movements, see [`unit_msg::read_unit_movements`]).
    UnitMovements(Vec<(u32, Movement)>),
    UnitPoints { obj_id: u32, hp: i32, mp: i32 },
    EnvDamage(crate::combat_status::EnvironmentDamage),
    SetBreath { breath: u32 },
    /// Received local dive/surface report. No Lua event or breathing timer is synthesized.
    UnderWater { start: bool },
    /// World-level report; raw state and timestamp bits, independent of own spawn.
    ConflictZoneState(crate::conflict::ConflictZoneState),
    ForceAttackSet { obj_id: u32, on: bool },
    UnitInvisible { obj_id: u32, invisible: bool },
    UnitDeath {
        obj_id: u32,
        /// `KillReason`.
        kill_reason: u8,
        /// How long until the unit can resurrect, ms.
        resurrect_wait_ms: i32,
        lost_exp: i32,
        durability_loss_rate: u8,
        /// Object id and name of the killer, when a unit did it.
        killer: Option<(u32, String)>,
    },
    /// `SCNotifyResurrection`: an in-place resurrection offer from this skill caster.
    ResurrectionOffered { caster: BuffCaster },
    CharacterResurrected { obj_id: u32, pos: Pos, yaw: f32 },
    /// `SCLootableState`: the unit or doodad has loot for the player (`has_loot`) or no longer has.
    Lootable { owner_type: u16, obj_id: u32, has_loot: bool },
    /// `SCUnitLootingState`: a unit started (`looting` 1) or stopped looting.
    UnitLooting { obj_id: u32, looting: u8 },
    /// `SCLootBagData`: the items left in the loot the player opened (the owner is in each item id).
    LootBag { items: Vec<Item>, loot_all: bool },
    /// `SCLootItemTook`: the player took an item out of the loot of `owner_obj_id`.
    LootItemTook { template_id: u32, item_index: u16, owner_type: u16, owner_obj_id: u32, count: i32 },
    /// `SCLootItemFailed`: the server did not hand the item over (`error`: an `ErrorMessageType`).
    LootItemFailed { error: i32, item_index: u16, owner_type: u16, owner_obj_id: u32, template_id: u32 },
    /// `SCTeleportUnit`: `yaw` is the lane's `z2` (radians).
    Teleport { reason: u8, error: i16, pos: Pos, yaw: f32 },
    /// Loading directive only; no inferred instance identity, completed load or automatic ACK.
    LoadInstance(crate::world_msg::InstanceLoad),
    /// Movement directive only; distinct from a completed transform or `SCTeleportUnit`.
    BlinkUnit(crate::world_msg::BlinkUnit),
    Gimmick(crate::gimmick::GimmickEvent),
    Housing(crate::housing::HousingEvent),
    Farm(crate::farm::FarmEvent),
    Coffer(crate::coffer::CofferEvent),
    Radar(crate::radar::RadarEvent),
    // -- progression and combat state
    /// `delta` experience gained (negative: lost) by the unit.
    ExpChanged { obj_id: u32, delta: i32, add_ability_exp: bool },
    /// The ability's experience is now `exp` (a total, not a delta).
    AbilityExpChanged { obj_id: u32, ability: u8, exp: i32 },
    RecoverableExp { obj_id: u32, recoverable: i32, penaltied: i32, reason: i32 },
    LevelChanged { obj_id: u32, level: u8 },
    AbilitySwapped { obj_id: u32, old_ability: u8, ability: u8 },
    /// Labor points added (`amount` > 0) or spent for `action` (an actability id, 0 for none).
    LaborPowerChanged { amount: i32, action: i32, point: i32, step: u8 },
    /// Honor and vocation (living) points, sent when the character is selected.
    GamePoints { honor: i32, vocation: i32, other: [i32; 8] },
    /// `kind` 0 honor, 1 vocation: `amount` was added.
    GamePointChanged { kind: u8, amount: i32 },
    /// Loyalty tokens.
    BmPoint(i64),
    /// `SCICSCashPoint`: the cash shop balance (credits).
    CashPoint(i32),
    CombatEngaged { obj_id: u32 },
    CombatCleared { obj_id: u32 },
    /// `SCCombatFirstHit`: `hitter` struck `victim` first (`hit_type` is the `SCUnitDamaged` hit type).
    CombatFirstHit { victim: u32, hitter: u32, hit_type: u32 },
    /// `SCAiAggro`: the hostile units on an NPC's aggro list (empty: the list was cleared).
    AiAggro { npc: u32, hostiles: Vec<AggroEntry> },
    /// `SCAggroTargetChanged`: the NPC `npc` now fights `target` (0: it went back to idle).
    AggroTargetChanged { npc: u32, target: u32 },
    /// `SCUnitAttached`: `obj_id` sits on `mount` (a mate, a vehicle, a ship) at attach point `point`.
    UnitAttached { obj_id: u32, mount: u32, point: u8, reason: u8 },
    /// `SCUnitDetached`: `obj_id` got off whatever it sat on.
    UnitDetached { obj_id: u32, reason: u8 },
    /// `SCUnitModelPostureChanged`: an NPC changed its idle pose.
    UnitPosture { obj_id: u32, posture: Posture },
    UnitNameChanged { obj_id: u32, name: String },
    UnitFactionChanged { obj_id: u32, name: String, faction_id: u32, faction_id2: u32, temporary: bool },
    // -- world: clock, weather, chat channels, targets
    /// The game clock in hours (0..24); `speed` (game hours per real second) comes with the
    /// detailed form only.
    TimeOfDay { hour: f32, speed: Option<f32> },
    Snow(bool),
    ChatJoined { chat_type: i16, sub_type: i16, faction_id: u32, name: String },
    ChatLeft { chat_type: i16, sub_type: i16, faction_id: u32 },
    /// `target_obj_id` 0 clears the target.
    TargetChanged { obj_id: u32, target_obj_id: u32 },
    // -- action bar, bag and equipment
    /// All hotbar slots (`SCActionSlots`).
    ActionSlots(Vec<ActionSlot>),
    /// Bag and bank sizes, ahead of their contents.
    InvenInit { bag_slots: u32, bank_slots: u32 },
    /// Ten slots of the bag or bank.
    InvenContents(InvenChunk),
    /// Item changes: loot, use, move, money ...
    ItemTasks(ItemTasks),
    ItemLifespan(crate::items::ItemLifespan),
    ItemDetailUpdated(crate::items::ItemDetailUpdate),
    ItemOperationError(crate::items::ItemOperationError),
    /// Items of a unit's equipment slots changed (`None` = unequipped).
    EquipmentChanged { obj_id: u32, items: Vec<(u8, Option<Item>)> },
    /// Native SC095 changes persistent IDs only, preserving other record fields.
    EquipmentIdsChanged { obj_id: u32, ids: [u64; 28] },
    /// Raw native specialty sale ratio receipt; zero is an error/reset path.
    SpecialtyRatio(i32),
    /// A container grew to `slots`.
    InvenExpanded { container: u8, slots: u8 },
    // -- quests
    /// The player's active quests (when the character spawns).
    Quests(Vec<Quest>),
    /// The completed quests, as blocks of 64 flags.
    CompletedQuests(Vec<CompletedBlock>),
    QuestStarted(Quest),
    QuestUpdated(Quest),
    /// Quest `quest_id` was handed in; `block` is its updated block of completed flags.
    QuestCompleted { quest_id: u32, block: CompletedBlock },
    /// Quest `quest_id` may be taken again (its completed flag was cleared).
    QuestReset { quest_id: u32, block: CompletedBlock },
    // -- skills and combat
    SkillStarted(SkillStarted),
    SkillFired(SkillFired),
    SkillEnded { tl: u16 },
    SkillStopped { obj_id: u32, skill_id: u32 },
    CastingStopped { tl: u16, duration_ms: u32 },
    /// Received skill/tag cooldown lists for the current player; empty is not a reset.
    Cooldowns(crate::skill::SkillCooldowns),
    SkillCooldownReset { obj_id: u32, skill_id: u32, tag_id: u32, gcd: bool },
    SkillLearned { skill_id: u32, level: u8 },
    SkillUpgraded { skill_id: u32, level: u8 },
    SkillsReset { obj_id: u32, ability: u8 },
    UnitDamaged(UnitDamage),
    UnitHealed(UnitHeal),
    CombatText { source: u32, target: u32, text_type: u8 },
    PlotEvent(PlotEvent),
    PlotEnded { tl: u16 },
    /// `SCPlotCastingStopped` (`channeling` false) or `SCPlotChannelingStopped`.
    PlotStopped { tl: u16, duration_ms: u32, last_event: u8, channeling: bool },
    // -- buffs
    BuffCreated { obj_id: u32, buff: Buff },
    BuffRemoved { obj_id: u32, index: u32 },
    BuffUpdated { obj_id:u32, index:u32, stacks:i32, charged:i32 },
    /// Raw skill-effect feedback. Authoritative item updates are separate item-task receipts.
    ScheduleItems { declared_count:u8, items:Vec<crate::schedule::ScheduleItem> },
    ScheduleItemSent { schedule_type:u32, raw_by_mail:u8 },
    ItemUccDataChanged { ucc_id:u64, player_id:u32, target_item_id:u64 },
    GradeEnchantResult(crate::item_effect::GradeEnchantResult),
    GradeEnchantBroadcast(crate::item_effect::GradeEnchantBroadcast),
    LunagemSocketingResult { raw_result:u8, item_id:u64, template_id:u32, raw_install:u8 },
    LunastoneSocketingResult { raw_result:u8, item_id:u64, template_id:u32 },
    /// A passive buff was learned (characters).
    BuffLearned { obj_id: u32, buff_id: u32 },
    DoodadsCreated(Vec<Doodad>),
    DoodadsRemoved(Vec<u32>),
    DoodadChanged { obj_id: u32, data: i32 },
    DoodadOriginator { obj_id: u32, new_owner_id: u32, new_faction: u32 },
    DoodadSound { obj_id: u32, sound_id: u32 },
    /// `SCVegetationCutdowning`: `unit_obj_id` cut the tree doodad `doodad_obj_id` down.
    VegetationCutdown { unit_obj_id: u32, doodad_obj_id: u32 },
    DoodadPhaseChanged { obj_id: u32, func_group_id: u32, time_left: u32, puzzle_group: i32, item_template_id: u32 },
    Chat(ChatMessage),
    ErrorMsg { code: i16, kind: u32, notify: bool },
    Kicked { reason: u8, message: String },
    // -- leaving
    PrepareLeaveWorld { time_ms: i32, target: u8, idle_kick: bool },
    LeaveWorldGranted { target: u8 },
    LeaveWorldCanceled,
    // -- anything else
    /// A compressed batch (`DD04`) of server packets (skill results), decoded one by one. The
    /// session reports each as if it had arrived alone.
    Batch(Vec<ServerMsg>),
    /// NPC function lists, doodad quest offers, NPC chat ... (see [`crate::interaction`]).
    Interaction(crate::interaction::InteractEvent),
    Shop(crate::shop::ShopEvent),
    Party(crate::party::PartyEvent),
    LootRoll(crate::loot_roll::LootRollEvent),
    Battlefield(crate::battlefield::BattlefieldEvent),
    ScheduledEventStarted(crate::scheduled::ScheduledEventStarted),
    Dominion(crate::DominionEvent),
    Trial(crate::trial::TrialEvent),
    Crime(crate::crime::CrimeEvent),
    Emote(crate::emote::EmoteEvent),
    Appellation(crate::appellation::AppellationEvent),
    Portal(crate::portal::PortalEvent),
    Duel(crate::duel::DuelEvent),
    Bounty(crate::BountyEvent),
    Ranking(crate::RankingEvent),
    Achievement(crate::achievement::AchievementEvent),
    Expedition(crate::expedition::ExpeditionEvent),
    Family(crate::family::FamilyEvent),
    Salon(crate::SalonEvent),
    /// Server notices, siege / tower defense / arrest messages, voice cues ... ([`crate::notice`]).
    Notice(crate::NoticeEvent),
    /// Knockback, weapon set, idle status, gear visibility of a unit ([`crate::unit_fx`]).
    UnitFx(crate::UnitFxEvent),
    /// Account configuration and lobby replies ([`crate::account`]).
    Account(crate::AccountEvent),
    CashShop(crate::CashShopEvent),
    /// `SCDiceValue`: someone in range rolled.
    Dice(crate::DiceRolled),
    ChatSpamDelay(crate::ChatSpamDelay),
    GameType(crate::proxy::GameType),
    UnitGmModeChanged(crate::unit_msg::UnitGmModeChanged),
    UnitPvpPointsChanged(crate::unit_msg::UnitPvpPointsChanged),
    Actability(crate::actability::ActabilityEvent),
    CraftFailureNotice(crate::craft::CraftFailureNotice),
    InitialConfig(crate::initial_config::InitialConfig),
    CraftItemUnlocked(crate::craft::CraftItemUnlock),
    Mate(crate::mate::MateEvent),
    Slave(crate::slave::SlaveEvent),
    Transport(crate::TransportReceipt),
    DoodadBond(crate::DoodadBondEvent),
    Auction(crate::auction::AuctionEvent),
    Trade(crate::trade::TradeEvent),
    Mail(crate::mail::MailEvent),
    Friend(crate::friend::FriendEvent),
    DecodeFailed { message:String },
    Unhandled { level: u8, opcode: u16, len: usize },
}

impl ServerMsg {
    pub fn name(&self) -> &'static str {
        match self {
            Self::ChangeState(_) => "ChangeState",
            Self::Pong(_) => "Pong",
            Self::EnterWorldResponse { .. } => "X2EnterWorldResponse",
            Self::ReconnectAuth { .. } => "SCReconnectAuth",
            Self::SlotCount(_) => "SCGetSlotCount",
            Self::AccountInfo { .. } => "SCAccountInfo",
            Self::PremiumPoint { .. } => "SCUpdatePremiumPoint",
            Self::RaceCongestion(_) => "SCRaceCongestion",
            Self::CharacterList { .. } => "SCCharacterList",
            Self::CharacterCreated(_) => "SCCreateCharacterResponse",
            Self::CharacterCreationFailed { .. } => "SCCharacterCreationFailed",
            Self::CharacterDeleteResponse { .. } => "SCDeleteCharacterResponse",
            Self::CancelCharacterDeleteResponse { .. } => "SCCancelCharacterDeleteResponse",
            Self::CharacterDeleted { .. } => "SCCharacterDeleted",
            Self::CharacterState(_) => "SCCharacterState",
            Self::UnitState(_) => "SCUnitState",
            Self::UnitsRemoved(_) => "SCUnitsRemoved",
            Self::UnitMoved { .. } => "SCOneUnitMovement",
            Self::UnitMovements(_) => "SCUnitMovements",
            Self::UnitPoints { .. } => "SCUnitPoints",
            Self::UnitDeath { .. } => "SCUnitDeath",
            Self::ResurrectionOffered { .. } => "SCNotifyResurrection",
            Self::CharacterResurrected { .. } => "SCCharacterResurrected",
            Self::Lootable { .. } => "SCLootableState",
            Self::UnitLooting { .. } => "SCUnitLootingState",
            Self::LootBag { .. } => "SCLootBagData",
            Self::LootItemTook { .. } => "SCLootItemTook",
            Self::LootItemFailed { .. } => "SCLootItemFailed",
            Self::ExpChanged { .. } => "SCExpChanged",
            Self::AbilityExpChanged { .. } => "SCAbilityExpChanged",
            Self::RecoverableExp { .. } => "SCRecoverableExp",
            Self::LevelChanged { .. } => "SCLevelChanged",
            Self::AbilitySwapped { .. } => "SCAbilitySwapped",
            Self::LaborPowerChanged { .. } => "SCCharacterLaborPowerChanged",
            Self::GamePoints { .. } => "SCCharacterGamePoints",
            Self::GamePointChanged { .. } => "SCGamePointChanged",
            Self::BmPoint(_) => "SCBmPoint",
            Self::CashPoint(_) => "SCICSCashPoint",
            Self::CombatEngaged { .. } => "SCCombatEngaged",
            Self::CombatCleared { .. } => "SCCombatCleared",
            Self::CombatFirstHit { .. } => "SCCombatFirstHit",
            Self::AiAggro { .. } => "SCAiAggro",
            Self::AggroTargetChanged { .. } => "SCAggroTargetChanged",
            Self::UnitAttached { .. } => "SCUnitAttached",
            Self::UnitDetached { .. } => "SCUnitDetached",
            Self::UnitPosture { .. } => "SCUnitModelPostureChanged",
            Self::UnitNameChanged { .. } => "SCUnitNameChanged",
            Self::UnitFactionChanged { .. } => "SCUnitFactionChanged",
            Self::TimeOfDay { speed: None, .. } => "SCTimeOfDay",
            Self::TimeOfDay { .. } => "SCDetailedTimeOfDay",
            Self::Snow(_) => "SCOnOffSnow",
            Self::ChatJoined { .. } => "SCJoinedChatChannel",
            Self::ChatLeft { .. } => "SCLeavedChatChannel",
            Self::TargetChanged { .. } => "SCTargetChanged",
            Self::ActionSlots(_) => "SCActionSlots",
            Self::InvenInit { .. } => "SCCharacterInvenInit",
            Self::InvenContents(_) => "SCCharacterInvenContents",
            Self::ItemTasks(_) => "SCItemTaskSuccess",
            Self::EquipmentChanged { .. } => "SCUnitEquipmentsChanged",
            Self::EquipmentIdsChanged { .. } => "SCUnitEquipmentIds",
            Self::SpecialtyRatio(_) => "SCSpecialtyRatio",
            Self::InvenExpanded { .. } => "SCInvenExpanded",
            Self::Quests(_) => "SCQuests",
            Self::CompletedQuests(_) => "SCCompletedQuests",
            Self::QuestStarted(_) => "SCQuestContextStarted",
            Self::QuestUpdated(_) => "SCQuestContextUpdated",
            Self::QuestCompleted { .. } => "SCQuestContextCompleted",
            Self::QuestReset { .. } => "SCQuestContextReset",
            Self::SkillStarted(_) => "SCSkillStarted",
            Self::SkillFired(_) => "SCSkillFired",
            Self::SkillEnded { .. } => "SCSkillEnded",
            Self::SkillStopped { .. } => "SCSkillStopped",
            Self::CastingStopped { .. } => "SCCastingStopped",
            Self::Cooldowns(_) => "SCCooldowns",
            Self::SkillCooldownReset { .. } => "SCSkillCooldownReset",
            Self::SkillLearned { .. } => "SCSkillLearned",
            Self::SkillUpgraded { .. } => "SCSkillUpgraded",
            Self::SkillsReset { .. } => "SCSkillsReset",
            Self::UnitDamaged(_) => "SCUnitDamaged",
            Self::UnitHealed(_) => "SCUnitHealed",
            Self::CombatText { .. } => "SCCombatText",
            Self::PlotEvent(_) => "SCPlotEvent",
            Self::PlotEnded { .. } => "SCPlotEnded",
            Self::PlotStopped { channeling: false, .. } => "SCPlotCastingStopped",
            Self::PlotStopped { .. } => "SCPlotChannelingStopped",
            Self::BuffCreated { .. } => "SCBuffCreated",
            Self::BuffRemoved { .. } => "SCBuffRemoved",
            Self::BuffUpdated { .. } => "SCBuffUpdated",
            Self::ScheduleItems { .. } => "SCScheduleItemUpdate",
            Self::ScheduleItemSent { .. } => "SCScheduleItemSent",
            Self::ItemUccDataChanged { .. } => "SCItemUccDataChanged",
            Self::GradeEnchantResult(_) => "SCItemGradeEnchantResult",
            Self::GradeEnchantBroadcast(_) => "SCItemGradeEnchantBroadcast",
            Self::LunagemSocketingResult { .. } => "SCItemSocketingLunagemResult",
            Self::LunastoneSocketingResult { .. } => "SCItemSocketingLunastoneResult",
            Self::BuffLearned { .. } => "SCBuffLearned",
            Self::Teleport { .. } => "SCTeleportUnit",
            Self::LoadInstance(_) => "SCLoadInstance",
            Self::BlinkUnit(_) => "SCBlinkUnit",
            Self::Gimmick(e) => e.name(),
            Self::Housing(e) => e.name(),
            Self::Farm(e) => e.name(),
            Self::Coffer(e) => e.name(),
            Self::Radar(e) => e.name(),
            Self::UiData(_) => "SCResponseUIData",
            Self::EnvDamage(_) => "SCEnvDamage",
            Self::SetBreath { .. } => "SCSetBreath",
            Self::UnderWater { .. } => "SCUnderWater",
            Self::ConflictZoneState(_) => "SCConflictZoneState",
            Self::ForceAttackSet { .. } => "SCForceAttackSet",
            Self::UnitInvisible { .. } => "SCUnitInvisible",
            Self::ItemLifespan(_) => "SCSyncItemLifespan",
            Self::ItemOperationError(_) => "SCItemOperationError",
            Self::ItemDetailUpdated(_) => "SCItemDetailUpdated",
            Self::DoodadsCreated(_) => "SCDoodad(s)Created",
            Self::DoodadsRemoved(_) => "SCDoodad(s)Removed",
            Self::DoodadChanged { .. } => "SCDoodadChanged",
            Self::DoodadOriginator { .. } => "SCDoodadOriginator",
            Self::DoodadSound { .. } => "SCDoodadSound",
            Self::VegetationCutdown { .. } => "SCVegetationCutdowning",
            Self::DoodadPhaseChanged { .. } => "SCDoodadPhaseChanged",
            Self::Chat(_) => "SCChatMessage",
            Self::ErrorMsg { .. } => "SCErrorMsg",
            Self::Kicked { .. } => "SCKicked",
            Self::PrepareLeaveWorld { .. } => "SCPrepareLeaveWorld",
            Self::LeaveWorldGranted { .. } => "SCLeaveWorldGranted",
            Self::LeaveWorldCanceled => "SCLeaveWorldCanceled",
            Self::Batch(_) => "CompressedGamePackets",
            Self::Interaction(e) => e.name(),
            Self::Shop(e) => e.name(),
            Self::Party(e) => e.name(),
            Self::LootRoll(e) => e.name(),
            Self::Battlefield(e) => e.name(),
            Self::ScheduledEventStarted(_) => "SCScheduledEventStarted",
            Self::Dominion(_) => "SCDominion",
            Self::Trial(_) => "SCTrial",
            Self::Crime(_) => "SCCrime",
            Self::Emote(e) => e.name(),
            Self::Appellation(e) => e.name(),
            Self::Portal(_) => "SCPortal",
            Self::Duel(e) => e.name(),
            Self::Bounty(_) => "SCBounty",
            Self::Ranking(_) => "SCRanking",
            Self::Achievement(_) => "SCAchievement",
            Self::Expedition(_) => "SCExpedition",
            Self::Family(_) => "SCFamily",
            Self::Salon(e) => e.name(),
            Self::Notice(e) => e.name(),
            Self::UnitFx(e) => e.name(),
            Self::Account(e) => e.name(),
            Self::CashShop(e) => e.name(),
            Self::Dice(_) => "SCDiceValue",
            Self::ChatSpamDelay(_) => "SCChatSpamDelay",
            Self::GameType(_) => "SetGameType",
            Self::UnitGmModeChanged(_) => "SCUnitGmModeChanged",
            Self::UnitPvpPointsChanged(_) => "SCUnitPvPPointsChanged",
            Self::Actability(e) => e.name(),
            Self::CraftFailureNotice(_) => "SCCraftFailed",
            Self::InitialConfig(_) => "SCInitialConfig",
            Self::CraftItemUnlocked(_) => "SCCraftItemUnlock",
            Self::Mate(e) => e.name(),
            Self::Slave(e) => e.name(),
            Self::Transport(e) => e.name(),
            Self::DoodadBond(e) => e.name(),
            Self::Auction(e) => e.name(),
            Self::Trade(e) => e.name(),
            Self::Mail(e) => e.name(),
            Self::Friend(e) => e.name(),
            Self::DecodeFailed { .. } => "decode failed",
            Self::Unhandled { .. } => "unhandled",
        }
    }

    /// Decodes a frame payload (starting at the `0xDD` byte).
    pub fn decode(frame: &[u8]) -> Result<Self> {
        let mut r = Reader::new(frame);
        let magic = r.u8()?;
        if magic != MAGIC {
            return Err(WireError::Invalid(format!("game frame starts with {magic:#04x}")));
        }
        let level = r.u8()?;
        match level {
            LEVEL_GAME => {
                r.skip(2)?; // hash, count
                let opcode = r.u16()?;
                Self::decode_game(opcode, &mut r, false)
            }
            LEVEL_PROXY => {
                let opcode = r.u16()?;
                Self::decode_proxy(opcode, &mut r)
            }
            LEVEL_COMPRESSED => {
                let count = r.u16()?;
                let raw = crate::compression::inflate_batch(r.rest())?;
                Ok(Self::Batch(Self::decode_batch(&raw, count)))
            }
            level => Ok(Self::Unhandled {
                level,
                opcode: 0,
                len: frame.len(),
            }),
        }
    }

    /// The packets of a `DD04` batch: `[u16 0][u16 opcode][body]` each. The bodies carry no length
    /// (the `u16` before the opcode is always 0, in the lane writer and in the C# it is ported from),
    /// so a packet this client cannot decode (unknown opcode, `Unhandled`; malformed body,
    /// `DecodeFailed`) is skipped by finding where the next one starts: the first later
    /// `[00 00][opcode]` from which the rest of the batch decodes completely, ending exactly at the
    /// end of the stream with the header's packet count. Without such a point the batch ends there.
    fn decode_batch(raw: &[u8], count: u16) -> Vec<Self> {
        let mut out = Vec::with_capacity(usize::from(count));
        let mut at = 0;
        while at < raw.len() {
            let (decoded, end) = Self::batch_entry(raw, at);
            let failed = match decoded {
                Ok(Self::Unhandled { opcode, .. }) => Self::Unhandled { level: LEVEL_COMPRESSED, opcode, len: 0 },
                Ok(m) => {
                    out.push(m);
                    at = end;
                    continue;
                }
                Err(e) => Self::DecodeFailed { message: e.to_string() },
            };
            let left = usize::from(count).saturating_sub(out.len() + 1);
            let resumed = (left > 0).then(|| Self::batch_resync(raw, at + 4, left)).flatten();
            let skipped = resumed.as_ref().map_or(raw.len(), |(next, _)| *next) - at;
            match failed {
                Self::Unhandled { level, opcode, .. } => out.push(Self::Unhandled { level, opcode, len: skipped.saturating_sub(4) }),
                failed => {
                    tracing::warn!("undecodable packet in a batch of {count}: {failed:?}");
                    out.push(failed);
                }
            }
            match resumed {
                Some((_, rest)) => out.extend(rest),
                None if left > 0 => tracing::warn!("batch of {count}: {left} packets after an undecodable one are lost"),
                None => {}
            }
            break;
        }
        out
    }

    /// One batch entry at `at`: the message and where the next entry starts.
    fn batch_entry(raw: &[u8], at: usize) -> (Result<Self>, usize) {
        let mut r = Reader::new(&raw[at..]);
        let decoded = (|| {
            r.skip(2)?;
            let opcode = r.u16()?;
            Self::decode_game(opcode, &mut r, true)
        })();
        (decoded, at + r.pos())
    }

    /// Where the batch resumes after an undecodable packet whose body starts at `from`: the one
    /// `[00 00][opcode]` from which exactly `left` packets decode and end the stream, with them.
    /// Two such points (a body can hold bytes that look like a packet start) are not guessed between,
    /// and session-control packets (enter world, reconnect, character list, kick, leave) never count:
    /// a false start must not move the session.
    fn batch_resync(raw: &[u8], from: usize, left: usize) -> Option<(usize, Vec<Self>)> {
        /// Candidate starts tried at most (bounds the work on a large batch).
        const MAX_TRIES: usize = 512;
        let tail = |start: usize| {
            let mut rest = Vec::with_capacity(left);
            let mut at = start;
            while rest.len() < left && at < raw.len() {
                if raw.get(at..at + 2) != Some(&[0, 0][..]) {
                    return None;
                }
                match Self::batch_entry(raw, at) {
                    (Ok(m), end) if m.batchable() => {
                        rest.push(m);
                        at = end;
                    }
                    _ => return None,
                }
            }
            (rest.len() == left && at == raw.len()).then_some((start, rest))
        };
        let candidates = (from..raw.len().saturating_sub(3)).filter(|&p| raw[p] == 0 && raw[p + 1] == 0);
        let mut found = candidates.take(MAX_TRIES).filter_map(tail);
        let first = found.next()?;
        found.next().is_none().then_some(first)
    }

    /// Whether a resynchronised batch may contain this message (see [`Self::batch_resync`]).
    fn batchable(&self) -> bool {
        !matches!(
            self,
            Self::Unhandled { .. }
                | Self::DecodeFailed { .. }
                | Self::EnterWorldResponse { .. }
                | Self::ReconnectAuth { .. }
                | Self::CharacterList { .. }
                | Self::CharacterState(_)
                | Self::Kicked { .. }
                | Self::PrepareLeaveWorld { .. }
                | Self::LeaveWorldGranted { .. }
        )
    }

    fn decode_proxy(opcode: u16, r: &mut Reader) -> Result<Self> {
        Ok(match opcode {
            pp::ChangeStatePacket => Self::ChangeState(r.i32()?),
            pp::SetGameTypePacket => Self::GameType(crate::proxy::GameType::read(r)?),
            pp::PongPacket => Self::Pong(crate::proxy::ProxyPong::read(r)?),
            opcode => Self::Unhandled {
                level: LEVEL_PROXY,
                opcode,
                len: r.remaining(),
            },
        })
    }

    fn decode_game(opcode: u16, r: &mut Reader, in_batch: bool) -> Result<Self> {
        Ok(match opcode {
            sc::X2EnterWorldResponsePacket => Self::EnterWorldResponse {
                reason: r.i16()?,
                gm: r.bool()?,
                stream_token: r.u32()?,
                stream_port: r.u16()?,
                world_time: r.i64()?,
            },
            sc::SCInitialConfigPacket => Self::InitialConfig(crate::initial_config::InitialConfig::read(r)?),
            sc::SCReconnectAuthPacket => Self::ReconnectAuth { token: r.u32()? },
            sc::SCGetSlotCountPacket => Self::SlotCount(r.u8()?),
            sc::SCAccountInfoPacket => Self::AccountInfo {
                pay_method: r.i32()?,
                pay_location: r.i32()?,
                pay_start: r.i64()?,
                pay_end: r.i64()?,
                real_pay_time: r.i64()?,
            },
            sc::SCUpdatePremiumPointPacket => Self::PremiumPoint {
                point: r.i32()?,
                old_grade: r.u8()?,
                grade: r.u8()?,
            },
            sc::SCRaceCongestionPacket => {
                let mut c = [0u8; 9];
                for v in &mut c {
                    *v = r.u8()?;
                }
                Self::RaceCongestion(c)
            }
            sc::SCCharacterListPacket => {
                let last = r.bool()?;
                let n = r.u8()?;
                let mut characters = Vec::with_capacity(n as usize);
                for _ in 0..n {
                    characters.push(Character::read(r)?);
                }
                Self::CharacterList { last, characters }
            }
            sc::SCCreateCharacterResponsePacket => {
                Self::CharacterCreated(Box::new(Character::read(r)?))
            }
            sc::SCCharacterCreationFailedPacket => Self::CharacterCreationFailed { reason: r.u8()? },
            sc::SCDeleteCharacterResponsePacket => Self::CharacterDeleteResponse {
                id: r.u32()?,
                status: r.u8()?,
                delete_request_time: r.i64()?,
                delete_time: r.i64()?,
            },
            sc::SCCancelCharacterDeleteResponsePacket => Self::CancelCharacterDeleteResponse {
                id: r.u32()?,
                status: r.u8()?,
            },
            sc::SCCharacterDeletedPacket => Self::CharacterDeleted {
                id: r.u32()?,
                name: r.string()?,
            },
            sc::SCCharacterStatePacket => Self::CharacterState(Box::new(CharacterState::read(r)?)),
            sc::SCUnitStatePacket => Self::UnitState(Box::new(if in_batch { Unit::read_in_batch(r)? } else { Unit::read(r)? })),
            sc::SCUnitsRemovedPacket => {
                let n = r.u16()?;
                let mut ids = Vec::with_capacity(n as usize);
                for _ in 0..n {
                    ids.push(r.bc()?);
                }
                Self::UnitsRemoved(ids)
            }
            sc::SCOneUnitMovementPacket => Self::UnitMoved {
                obj_id: r.bc()?,
                movement: Movement::read(r)?,
            },
            sc::SCUnitMovementsPacket => Self::UnitMovements(unit_msg::read_unit_movements(r)?),
            sc::SCCombatFirstHitPacket => Self::CombatFirstHit { victim: r.bc()?, hitter: r.bc()?, hit_type: r.u32()? },
            sc::SCAggroTargetChangedPacket => Self::AggroTargetChanged { npc: r.bc()?, target: r.bc()? },
            sc::SCAiAggroPacket => {
                let (npc, hostiles) = unit_msg::read_ai_aggro(r)?;
                Self::AiAggro { npc, hostiles }
            }
            sc::SCUnitAttachedPacket => {
                let obj_id = r.bc()?;
                let point = r.u8()?;
                Self::UnitAttached { obj_id, point, mount: r.bc()?, reason: r.u8()? }
            }
            sc::SCUnitDetachedPacket => Self::UnitDetached { obj_id: r.bc()?, reason: r.u8()? },
            sc::SCUnitModelPostureChangedPacket => Self::UnitPosture { obj_id: r.bc()?, posture: Posture::read(r)? },
            sc::SCUnitPointsPacket => Self::UnitPoints {
                obj_id: r.bc()?,
                hp: r.i32()? / 100,
                mp: r.i32()? / 100,
            },
            sc::SCUnitDeathPacket => {
                let obj_id = r.bc()?;
                let kill_reason = r.u8()?;
                let resurrect_wait_ms = r.i32()?;
                let lost_exp = r.i32()?;
                let durability_loss_rate = r.u8()?;
                let killer_id = r.bc()?;
                let killer = if killer_id != 0 {
                    // game type, kill streak, three flags, then the killer's name.
                    r.skip(1 + 2 + 3)?;
                    Some((killer_id, r.string()?))
                } else {
                    None
                };
                Self::UnitDeath { obj_id, kill_reason, resurrect_wait_ms, lost_exp, durability_loss_rate, killer }
            }
            sc::SCNotifyResurrectionPacket => Self::ResurrectionOffered { caster: BuffCaster::read(r)? },
            sc::SCCharacterResurrectedPacket => Self::CharacterResurrected {
                obj_id: r.bc()?,
                pos: r.position()?,
                yaw: r.f32()?,
            },
            sc::SCLootableStatePacket => {
                r.skip(2)?; // an item index, unused
                let owner_type = r.u16()?;
                let obj_id = r.bc()?;
                r.skip(1)?;
                Self::Lootable { owner_type, obj_id, has_loot: r.bool()? }
            }
            sc::SCUnitLootingStatePacket => Self::UnitLooting { obj_id: r.bc()?, looting: r.u8()? },
            sc::SCLootBagDataPacket => {
                let n = r.u8()?;
                let mut items = Vec::with_capacity(usize::from(n));
                for _ in 0..n {
                    items.extend(Item::read(r)?);
                }
                Self::LootBag { items, loot_all: r.bool()? }
            }
            sc::SCLootItemTookPacket => {
                let template_id = r.u32()?;
                let item_index = r.u16()?;
                let owner_type = r.u16()?;
                let owner_obj_id = r.bc()?;
                r.skip(1)?;
                Self::LootItemTook { template_id, item_index, owner_type, owner_obj_id, count: r.i32()? }
            }
            sc::SCLootItemFailedPacket => {
                let error = r.i32()?;
                let item_index = r.u16()?;
                let owner_type = r.u16()?;
                let owner_obj_id = r.bc()?;
                r.skip(1)?;
                Self::LootItemFailed { error, item_index, owner_type, owner_obj_id, template_id: r.u32()? }
            }
            sc::SCExpChangedPacket => Self::ExpChanged {
                obj_id: r.bc()?,
                delta: r.i32()?,
                add_ability_exp: r.bool()?,
            },
            sc::SCAbilityExpChangedPacket => Self::AbilityExpChanged {
                obj_id: r.bc()?,
                ability: r.u8()?,
                exp: r.i32()?,
            },
            sc::SCRecoverableExpPacket => Self::RecoverableExp {
                obj_id: r.bc()?,
                recoverable: r.i32()?,
                penaltied: r.i32()?,
                reason: r.i32()?,
            },
            sc::SCLevelChangedPacket => Self::LevelChanged { obj_id: r.bc()?, level: r.u8()? },
            sc::SCAbilitySwappedPacket => Self::AbilitySwapped {
                obj_id: r.bc()?,
                old_ability: r.u8()?,
                ability: r.u8()?,
            },
            sc::SCCharacterLaborPowerChangedPacket => Self::LaborPowerChanged {
                amount: r.i32()?,
                action: r.i32()?,
                point: r.i32()?,
                step: r.u8()?,
            },
            sc::SCCharacterGamePointsPacket => {
                let honor = r.i32()?;
                let vocation = r.i32()?;
                let mut other = [0i32; 8];
                for v in &mut other {
                    *v = r.i32()?;
                }
                Self::GamePoints { honor, vocation, other }
            }
            sc::SCGamePointChangedPacket => Self::GamePointChanged { kind: r.u8()?, amount: r.i32()? },
            sc::SCBmPointPacket => Self::BmPoint(r.i64()?),
            sc::SCICSCashPointPacket => Self::CashPoint(r.i32()?),
            sc::SCCombatEngagedPacket => Self::CombatEngaged { obj_id: r.bc()? },
            sc::SCCombatClearedPacket => Self::CombatCleared { obj_id: r.bc()? },
            sc::SCUnitNameChangedPacket => Self::UnitNameChanged { obj_id: r.bc()?, name: r.string()? },
            sc::SCUnitFactionChangedPacket => Self::Expedition(crate::expedition::ExpeditionEvent::read_lane(opcode,r)?),
            sc::SCTimeOfDayPacket => Self::TimeOfDay { hour: r.f32()?, speed: None },
            sc::SCDetailedTimeOfDayPacket => {
                let hour = r.f32()?;
                let speed = r.f32()?;
                // Day start and end (0 and 24).
                r.skip(8)?;
                Self::TimeOfDay { hour, speed: Some(speed) }
            }
            sc::SCOnOffSnowPacket => Self::Snow(r.bool()?),
            sc::SCLoadInstancePacket => Self::LoadInstance(crate::world_msg::InstanceLoad::read(r)?),
            sc::SCBlinkUnitPacket => Self::BlinkUnit(crate::world_msg::BlinkUnit::read(r)?),
            sc::SCSyncItemLifespanPacket => Self::ItemLifespan(crate::items::ItemLifespan::read(r)?),
            sc::SCItemOperationErrorPacket => Self::ItemOperationError(crate::items::ItemOperationError::read(r)?),
            sc::SCItemDetailUpdatedPacket => Self::ItemDetailUpdated(crate::items::ItemDetailUpdate::read(r)?),
            sc::SCResponseUIDataPacket => Self::UiData(crate::ui_data::UiDataResponse::read(r)?),
            sc::SCEnvDamagePacket => Self::EnvDamage(crate::combat_status::EnvironmentDamage::read(r)?),
            sc::SCSetBreathPacket => Self::SetBreath { breath: r.u32()? },
            sc::SCUnderWaterPacket => Self::UnderWater { start: r.bool()? },
            sc::SCConflictZoneStatePacket => Self::ConflictZoneState(crate::conflict::ConflictZoneState::read(r)?),
            sc::SCForceAttackSetPacket => Self::ForceAttackSet { obj_id: r.bc()?, on: r.bool()? },
            sc::SCUnitInvisiblePacket => Self::UnitInvisible { obj_id: r.bc()?, invisible: r.bool()? },
            opcode if crate::radar::RadarEvent::handles(opcode) => Self::Radar(crate::radar::RadarEvent::read(opcode, r)?),
            opcode if crate::housing::HousingEvent::handles(opcode) => Self::Housing(crate::housing::HousingEvent::read(opcode, r)?),
            opcode if crate::farm::FarmEvent::handles(opcode) => Self::Farm(crate::farm::FarmEvent::read(opcode, r)?),
            opcode if crate::coffer::CofferEvent::handles(opcode) => Self::Coffer(crate::coffer::CofferEvent::read(opcode, r)?),
            opcode if crate::gimmick::GimmickEvent::handles(opcode) => Self::Gimmick(crate::gimmick::GimmickEvent::read(opcode, r)?),
            sc::SCJoinedChatChannelPacket => Self::ChatJoined {
                chat_type: r.i16()?,
                sub_type: r.i16()?,
                faction_id: r.u32()?,
                name: r.string()?,
            },
            sc::SCLeavedChatChannelPacket => Self::ChatLeft {
                chat_type: r.i16()?,
                sub_type: r.i16()?,
                faction_id: r.u32()?,
            },
            sc::SCTargetChangedPacket => Self::TargetChanged { obj_id: r.bc()?, target_obj_id: r.bc()? },
            sc::SCActionSlotsPacket => Self::ActionSlots(crate::items::read_action_slots(r)?),
            sc::SCCharacterInvenInitPacket => Self::InvenInit { bag_slots: r.u32()?, bank_slots: r.u32()? },
            sc::SCCharacterInvenContentsPacket => {
                let chunk = InvenChunk::read(r)?;
                if chunk.truncated { return Err(WireError::Invalid("truncated inventory chunk".into())); }
                Self::InvenContents(chunk)
            }
            sc::SCItemTaskSuccessPacket => {
                let tasks = ItemTasks::read(r)?;
                if tasks.truncated { return Err(WireError::Invalid("truncated item tasks".into())); }
                r.u32()?; // type(id), present even when there are no tasks / forced removals
                Self::ItemTasks(tasks)
            }
            sc::SCUnitEquipmentsChangedPacket => {
                let obj_id = r.bc()?;
                let n = r.u8()?;
                let mut items = Vec::with_capacity(usize::from(n));
                for _ in 0..n {
                    items.push((r.u8()?, Item::read(r)?));
                }
                Self::EquipmentChanged { obj_id, items }
            }
            sc::SCSpecialtyRatioPacket => Self::SpecialtyRatio(r.i32()?),
            sc::SCUnitEquipmentIdsPacket => {
                let obj_id=r.bc()?;
                let mut ids=[0;28];
                for id in &mut ids {*id=r.u64()?;}
                Self::EquipmentIdsChanged{obj_id,ids}
            }
            sc::SCInvenExpandedPacket => Self::InvenExpanded { container: r.u8()?, slots: r.u8()? },
            sc::SCQuestsPacket => Self::Quests(quest::read_quests(r)?),
            sc::SCCompletedQuestsPacket => Self::CompletedQuests(quest::read_completed(r)?),
            sc::SCQuestContextStartedPacket => Self::QuestStarted(quest::read_context(r)?),
            sc::SCQuestContextUpdatedPacket => Self::QuestUpdated(quest::read_context(r)?),
            sc::SCQuestContextCompletedPacket | sc::SCQuestContextResetPacket => {
                let quest_id = r.u32()?;
                let bytes: [u8; 8] = r.bytes(8)?.try_into().expect("8 bytes");
                let block = CompletedBlock { id: CompletedBlock::block_of(quest_id), bits: u64::from_le_bytes(bytes) };
                if opcode == sc::SCQuestContextCompletedPacket {
                    Self::QuestCompleted { quest_id, block }
                } else {
                    Self::QuestReset { quest_id, block }
                }
            }
            sc::SCSkillStartedPacket => Self::SkillStarted(SkillStarted::read(r)?),
            sc::SCSkillFiredPacket => Self::SkillFired(SkillFired::read(r)?),
            sc::SCSkillEndedPacket => Self::SkillEnded { tl: r.u16()? },
            sc::SCSkillStoppedPacket => Self::SkillStopped { obj_id: r.bc()?, skill_id: r.u32()? },
            sc::SCCastingStoppedPacket => Self::CastingStopped { tl: r.u16()?, duration_ms: r.u32()? },
            sc::SCCooldownsPacket => Self::Cooldowns(crate::skill::SkillCooldowns::read(r)?),
            sc::SCSkillCooldownResetPacket => Self::SkillCooldownReset {
                obj_id: r.bc()?,
                skill_id: r.u32()?,
                tag_id: r.u32()?,
                gcd: r.bool()?,
            },
            sc::SCSkillLearnedPacket => Self::SkillLearned { skill_id: r.u32()?, level: r.u8()? },
            sc::SCSkillUpgradedPacket => Self::SkillUpgraded { skill_id: r.u32()?, level: r.u8()? },
            sc::SCSkillsResetPacket => Self::SkillsReset { obj_id: r.bc()?, ability: r.u8()? },
            sc::SCUnitDamagedPacket => Self::UnitDamaged(UnitDamage::read(r)?),
            sc::SCUnitHealedPacket => Self::UnitHealed(UnitHeal::read(r)?),
            sc::SCCombatTextPacket => Self::CombatText { source: r.bc()?, target: r.bc()?, text_type: r.u8()? },
            sc::SCPlotEventPacket => Self::PlotEvent(PlotEvent::read(r)?),
            sc::SCPlotEndedPacket => Self::PlotEnded { tl: r.u16()? },
            sc::SCPlotCastingStoppedPacket | sc::SCPlotChannelingStoppedPacket => Self::PlotStopped {
                tl: r.u16()?,
                duration_ms: r.u32()?,
                last_event: r.u8()?,
                channeling: opcode == sc::SCPlotChannelingStoppedPacket,
            },
            sc::SCBuffCreatedPacket => {
                let (obj_id, buff) = Buff::read_created(r)?;
                Self::BuffCreated { obj_id, buff }
            }
            sc::SCBuffRemovedPacket => Self::BuffRemoved { obj_id: r.bc()?, index: r.u32()? },
            sc::SCBuffUpdatedPacket => Self::BuffUpdated { obj_id:r.bc()?, index:r.u32()?, stacks:r.i32()?, charged:r.i32()? },
            sc::SCScheduleItemUpdatePacket => { let (declared_count,items)=crate::schedule::read_lane(r)?; Self::ScheduleItems{declared_count,items} },
            sc::SCScheduleItemSentPacket => Self::ScheduleItemSent{schedule_type:r.u32()?,raw_by_mail:r.u8()?},
            sc::SCItemUccDataChangedPacket => Self::ItemUccDataChanged { ucc_id:r.u64()?, player_id:r.u32()?, target_item_id:r.u64()? },
            sc::SCItemGradeEnchantResultPacket => Self::GradeEnchantResult(crate::item_effect::GradeEnchantResult::read(r)?),
            sc::SCItemGradeEnchantBroadcastPacket => Self::GradeEnchantBroadcast(crate::item_effect::GradeEnchantBroadcast::read(r)?),
            sc::SCItemSocketingLunagemResultPacket => Self::LunagemSocketingResult { raw_result:r.u8()?, item_id:r.u64()?, template_id:r.u32()?, raw_install:r.u8()? },
            sc::SCItemSocketingLunastoneResultPacket => Self::LunastoneSocketingResult { raw_result:r.u8()?, item_id:r.u64()?, template_id:r.u32()? },
            sc::SCBuffLearnedPacket => Self::BuffLearned { obj_id: r.bc()?, buff_id: r.u32()? },
            sc::SCTeleportUnitPacket => Self::Teleport {
                reason: r.u8()?,
                error: r.i16()?,
                pos: r.position()?,
                yaw: r.f32()?,
            },
            sc::SCDoodadCreatedPacket => Self::DoodadsCreated(vec![Doodad::read(r)?]),
            sc::SCDoodadsCreatedPacket => {
                let n = r.u8()?;
                let mut doodads = Vec::with_capacity(n as usize);
                for _ in 0..n {
                    doodads.push(Doodad::read(r)?);
                }
                Self::DoodadsCreated(doodads)
            }
            sc::SCDoodadRemovedPacket => Self::DoodadsRemoved(vec![r.bc()?]),
            sc::SCDoodadChangedPacket => Self::DoodadChanged { obj_id: r.bc()?, data: r.i32()? },
            sc::SCDoodadOriginatorPacket => Self::DoodadOriginator { obj_id: r.bc()?, new_owner_id: r.u32()?, new_faction: r.u32()? },
            sc::SCDoodadSoundPacket => Self::DoodadSound { obj_id: r.bc()?, sound_id: r.u32()? },
            sc::SCVegetationCutdowningPacket => Self::VegetationCutdown { unit_obj_id: r.bc()?, doodad_obj_id: r.bc()? },
            sc::SCDoodadsRemovedPacket => {
                let n = r.u16()?;
                r.skip(1)?; // last
                let mut ids = Vec::with_capacity(n as usize);
                for _ in 0..n {
                    ids.push(r.bc()?);
                    r.skip(1)?;
                }
                Self::DoodadsRemoved(ids)
            }
            sc::SCDoodadPhaseChangedPacket => Self::DoodadPhaseChanged {
                obj_id: r.bc()?,
                func_group_id: r.u32()?,
                time_left: r.u32()?,
                puzzle_group: r.i32()?,
                item_template_id: r.u32()?,
            },
            sc::SCChatMessagePacket => Self::Chat(ChatMessage::read(r)?),
            sc::SCErrorMsgPacket => {
                let code = r.i16()?;
                r.skip(2)?;
                Self::ErrorMsg {
                    code,
                    kind: r.u32()?,
                    notify: r.bool()?,
                }
            }
            sc::SCKickedPacket => Self::Kicked {
                reason: r.u8()?,
                message: r.string()?,
            },
            sc::SCPrepareLeaveWorldPacket => Self::PrepareLeaveWorld {
                time_ms: r.i32()?,
                target: r.u8()?,
                idle_kick: r.bool()?,
            },
            sc::SCLeaveWorldGrantedPacket => Self::LeaveWorldGranted { target: r.u8()? },
            sc::SCLeaveWorldCanceledPacket => Self::LeaveWorldCanceled,
            opcode if crate::interaction::InteractEvent::handles(opcode) => Self::Interaction(crate::interaction::InteractEvent::read(opcode, r)?),
            opcode if crate::shop::ShopEvent::handles(opcode) => Self::Shop(crate::shop::ShopEvent::read(r)?),
            opcode if crate::party::PartyEvent::handles(opcode) => Self::Party(crate::party::PartyEvent::read(opcode, r)?),
            opcode if crate::loot_roll::LootRollEvent::handles(opcode) => Self::LootRoll(crate::loot_roll::LootRollEvent::read(opcode, r)?),
            sc::SCCraftItemUnlockPacket => Self::CraftItemUnlocked(crate::craft::CraftItemUnlock::read(r)?),
            sc::SCCraftFailedPacket => Self::CraftFailureNotice(crate::craft::CraftFailureNotice::read(r)?),
            sc::SCUnitPvPPointsChangedPacket => Self::UnitPvpPointsChanged(crate::unit_msg::UnitPvpPointsChanged::read(r)?),
            sc::SCUnitGmModeChangedPacket => Self::UnitGmModeChanged(crate::unit_msg::UnitGmModeChanged::read(r)?),
            opcode if crate::DominionEvent::handles(opcode) => Self::Dominion(crate::DominionEvent::read_lane(opcode,r)?),
            opcode if crate::trial::TrialEvent::handles(opcode) => Self::Trial(crate::trial::TrialEvent::read(opcode,r)?),
            opcode if crate::RankingEvent::handles(opcode) => Self::Ranking(crate::RankingEvent::read(opcode,r)?),
            opcode if crate::BountyEvent::handles(opcode) => Self::Bounty(crate::BountyEvent::read(opcode,r)?),
            opcode if crate::duel::DuelEvent::handles(opcode) => Self::Duel(crate::duel::DuelEvent::read(opcode, r)?),
            opcode if crate::emote::EmoteEvent::handles(opcode) => Self::Emote(crate::emote::EmoteEvent::read(opcode, r)?),
            opcode if crate::appellation::AppellationEvent::handles(opcode) => Self::Appellation(crate::appellation::AppellationEvent::read(opcode, r)?),
            opcode if crate::crime::CrimeEvent::handles(opcode) => Self::Crime(crate::crime::CrimeEvent::read(opcode,r)?),
            opcode if crate::portal::PortalEvent::handles(opcode) => Self::Portal(crate::portal::PortalEvent::read(opcode, r)?),
            opcode if crate::achievement::AchievementEvent::handles(opcode) => Self::Achievement(crate::achievement::AchievementEvent::read_lane(opcode,r)?),
            opcode if crate::expedition::ExpeditionEvent::handles(opcode) => Self::Expedition(crate::expedition::ExpeditionEvent::read_lane(opcode,r)?),
            0x02d..=0x037 => Self::Family(crate::family::FamilyEvent::read_lane(opcode,r)?),
            opcode if crate::SalonEvent::handles(opcode) => Self::Salon(crate::SalonEvent::read(opcode, r)?),
            opcode if crate::NoticeEvent::handles(opcode) => Self::Notice(crate::NoticeEvent::read(opcode, r)?),
            opcode if crate::UnitFxEvent::handles(opcode) => Self::UnitFx(crate::UnitFxEvent::read(opcode, r)?),
            opcode if crate::AccountEvent::handles(opcode) => Self::Account(crate::AccountEvent::read(opcode, r)?),
            opcode if crate::CashShopEvent::handles(opcode) => Self::CashShop(crate::CashShopEvent::read(opcode, r)?),
            opcode if crate::DiceRolled::handles(opcode) => Self::Dice(crate::DiceRolled::read(r)?),
            sc::SCChatSpamDelayPacket => Self::ChatSpamDelay(crate::ChatSpamDelay::read(r)?),
            sc::SCScheduledEventStartedPacket => Self::ScheduledEventStarted(crate::scheduled::ScheduledEventStarted::read_native(r)?),
            opcode if crate::battlefield::BattlefieldEvent::handles(opcode) => Self::Battlefield(crate::battlefield::BattlefieldEvent::read(opcode, r)?),
            opcode if crate::actability::ActabilityEvent::handles(opcode) => Self::Actability(crate::actability::ActabilityEvent::read(opcode, r)?),
            opcode if crate::DoodadBondEvent::handles(opcode) => Self::DoodadBond(crate::DoodadBondEvent::read(opcode, r)?),
            opcode if crate::slave::SlaveEvent::handles(opcode) => Self::Slave(crate::slave::SlaveEvent::read(opcode, r)?),
            opcode if crate::TransportReceipt::handles(opcode) => Self::Transport(crate::TransportReceipt::read(opcode, r)?),
            opcode if crate::mate::MateEvent::handles(opcode) => Self::Mate(crate::mate::MateEvent::read(opcode, r)?),
            opcode if crate::auction::AuctionEvent::handles(opcode) => Self::Auction(crate::auction::AuctionEvent::read(opcode, r)?),
            opcode if crate::trade::TradeEvent::handles(opcode) => Self::Trade(crate::trade::TradeEvent::read(opcode, r)?),
            opcode if crate::mail::MailEvent::handles(opcode) => Self::Mail(crate::mail::MailEvent::read(opcode, r)?),
            opcode if crate::friend::FriendEvent::handles(opcode) => Self::Friend(crate::friend::FriendEvent::read(opcode, r)?),
            opcode => Self::Unhandled {
                level: LEVEL_GAME,
                opcode,
                len: r.remaining(),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{race, gender, slot};

    #[test]
    fn level1_header_matches_the_server_layout() {
        // `[dd][01][hash][count][u16 opcode][body]`, see the server's `net::packet` tests.
        let m = ClientMsg::NotifyInGame.encode(7);
        assert_eq!(m, [0xDD, 0x01, 0x00, 0x07, 0x29, 0x00]);
        // Proxy packets have no hash / count.
        assert_eq!(ClientMsg::FinishState(3).encode(7), [0xDD, 0x02, 0x01, 0x00, 3, 0, 0, 0]);
    }

    #[test]
    fn client_bodies_match_the_testclient() {
        // X2EnterWorld: pFrom, pTo, account, cookie, zone, tb, revision.
        let b = ClientMsg::EnterWorld { account_id: 3, cookie: 4 }.encode_body();
        assert_eq!(b.len(), 4 * 4 + 4 + 1 + 8);
        assert_eq!(&b[8..16], &[3, 0, 0, 0, 4, 0, 0, 0]);
        // CSListCharacter: i32 size, empty blob.
        assert_eq!(ClientMsg::ListCharacters.encode_body(), [0, 0, 0, 0, 0, 0]);
        // CSSelectCharacter: id, gm flag, unknown.
        assert_eq!(ClientMsg::SelectCharacter { id: 9 }.encode_body(), [9, 0, 0, 0, 0, 0]);
        // CSSpawnCharacter with every option: flag 0x1F, 6 stp bytes, four bools.
        let mut want = vec![0x1F];
        want.extend([0u8; 6]);
        want.extend([1, 1, 1, 1]);
        assert_eq!(ClientMsg::SpawnCharacter(VisualOptions::default()).encode_body(), want);
        // CSLeaveWorld: the target byte.
        assert_eq!(ClientMsg::LeaveWorld(LeaveTarget::CharacterSelect).encode_body(), [1]);
        assert_eq!(ClientMsg::DeleteCharacter { id: 258 }.encode_body(), [2, 1, 0, 0]);
        assert_eq!(ClientMsg::ChangeTarget { obj_id: 0x010203 }.encode_body(), [3, 2, 1]);
        // CSSellBackpackGoods: the same bc NPC object id, the 1.2 capture's 4b 68 00.
        let sale = ClientMsg::SellBackpackGoods { obj_id: 0x684b };
        assert_eq!(sale.opcode(), 0x042);
        assert_eq!(sale.encode_body(), [0x4b, 0x68, 0x00]);
        // CSSendChatMessage: type, sub, i32, target, message, language, ability.
        let b = ClientMsg::Chat {
            chat_type: 0,
            target: String::new(),
            message: "hi".into(),
        }
        .encode_body();
        assert_eq!(b, [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, b'h', b'i', 0, 0, 0, 0, 0]);
        // Ping: tPhy, ping, local.
        let b = ClientMsg::Ping { time_ms: 258 }.encode_body();
        assert_eq!(b.len(), 20);
        assert_eq!(&b[..2], &[2, 1]);
        // CSTeleportEnded: two fixed point longs, height, identity quaternion.
        let b = ClientMsg::TeleportEnded { pos: Pos::new(1.0, 2.0, 3.0) }.encode_body();
        assert_eq!(b.len(), 8 + 8 + 4 + 12 + 4);
        assert_eq!(&b[b.len() - 4..], &[0, 0, 0x80, 0x3f]);
    }

    #[test]
    fn create_character_message_carries_the_request() {
        let req = CreateCharacter::new("Bob", race::ELF, gender::FEMALE);
        let b = ClientMsg::CreateCharacter(req).encode_body();
        assert_eq!(&b[..5], &[3, 0, b'B', b'o', b'b']);
        assert_eq!(&b[5..7], &[race::ELF, gender::FEMALE]);
    }

    fn game_frame(opcode: u16, body: &[u8]) -> Vec<u8> {
        let mut f = vec![MAGIC, LEVEL_GAME, 0, 0];
        f.extend(opcode.to_le_bytes());
        f.extend(body);
        f
    }

    #[test]
    fn server_frames_decode() {
        // Server layout: `[dd][01][00][00][u16 opcode][body]`.
        let mut w = Writer::new();
        w.i16(0).bool(false).u32(0xAABB).u16(27250).i64(1_700_000_000).u16(0).string("");
        let m = ServerMsg::decode(&game_frame(sc::X2EnterWorldResponsePacket, &w.finish())).unwrap();
        assert_eq!(
            m,
            ServerMsg::EnterWorldResponse {
                reason: 0,
                gm: false,
                stream_token: 0xAABB,
                stream_port: 27250,
                world_time: 1_700_000_000
            }
        );

        let m = ServerMsg::decode(&[MAGIC, LEVEL_PROXY, 0, 0, 5, 0, 0, 0]).unwrap();
        assert_eq!(m, ServerMsg::ChangeState(5));

        let m = ServerMsg::decode(&game_frame(sc::SCRaceCongestionPacket, &[0, 1, 2, 3, 0, 0, 0, 0, 0])).unwrap();
        assert_eq!(m, ServerMsg::RaceCongestion([0, 1, 2, 3, 0, 0, 0, 0, 0]));

        let m = ServerMsg::decode(&game_frame(sc::SCCharacterCreationFailedPacket, &[4])).unwrap();
        assert_eq!(m, ServerMsg::CharacterCreationFailed { reason: 4 });

        let m = ServerMsg::decode(&game_frame(sc::SCLeaveWorldGrantedPacket, &[1])).unwrap();
        assert_eq!(m, ServerMsg::LeaveWorldGranted { target: 1 });

        let mut w = Writer::new();
        w.i32(10_000).u8(0).bool(false);
        let m = ServerMsg::decode(&game_frame(sc::SCPrepareLeaveWorldPacket, &w.finish())).unwrap();
        assert_eq!(m, ServerMsg::PrepareLeaveWorld { time_ms: 10_000, target: 0, idle_kick: false });

        let mut w = Writer::new();
        w.u16(2).bc(0x101).bc(0x102);
        let m = ServerMsg::decode(&game_frame(sc::SCUnitsRemovedPacket, &w.finish())).unwrap();
        assert_eq!(m, ServerMsg::UnitsRemoved(vec![0x101, 0x102]));

        let mut w = Writer::new();
        w.bc(0x101).i32(5000).i32(200);
        let m = ServerMsg::decode(&game_frame(sc::SCUnitPointsPacket, &w.finish())).unwrap();
        assert_eq!(m, ServerMsg::UnitPoints { obj_id: 0x101, hp: 50, mp: 2 });

        let mut w = Writer::new();
        w.u16(1).bool(true).bc(0x303).bool(true);
        let m = ServerMsg::decode(&game_frame(sc::SCDoodadsRemovedPacket, &w.finish())).unwrap();
        assert_eq!(m, ServerMsg::DoodadsRemoved(vec![0x303]));

        let mut w = Writer::new();
        w.i16(95).i16(95).u32(0).bool(false);
        let m = ServerMsg::decode(&game_frame(sc::SCErrorMsgPacket, &w.finish())).unwrap();
        assert_eq!(m, ServerMsg::ErrorMsg { code: 95, kind: 0, notify: false });

        // Unknown packets are reported with their opcode, not an error.
        let m = ServerMsg::decode(&game_frame(0x7fff, &[1, 2, 3])).unwrap();
        assert_eq!(m, ServerMsg::Unhandled { level: LEVEL_GAME, opcode: 0x7fff, len: 3 });
        assert!(ServerMsg::decode(&[0x00, 0x01]).is_err());
        assert!(ServerMsg::decode(&game_frame(sc::SCUnitPointsPacket, &[1])).is_err());
    }

    fn decode(op: u16, body: &[u8]) -> ServerMsg {
        ServerMsg::decode(&game_frame(op, body)).unwrap()
    }

    #[test]
    fn resurrection_offers_decode_skill_casters_without_losing_the_next_packet() {
        use crate::buff::caster_kind;
        for kind in [caster_kind::UNIT, caster_kind::ITEM, caster_kind::MOUNT] {
            let mut body = Writer::new();
            body.u8(kind).bc(0x0A0B0C);
            match kind {
                // The server's notify_resurrection_writes_skill_caster golden values.
                caster_kind::ITEM => { body.u64(77).u32(8000).u8(2).u32(3); }
                caster_kind::MOUNT => { body.u32(9000); }
                _ => {}
            }
            let body = body.finish();
            let expected = ServerMsg::ResurrectionOffered { caster: BuffCaster { kind, obj_id: 0x0A0B0C } };
            assert_eq!(decode(sc::SCNotifyResurrectionPacket, &body), expected);
            let mut batch = Writer::new();
            batch.u16(0).u16(sc::SCNotifyResurrectionPacket).raw(&body);
            batch.u16(0).u16(sc::SCUnitPointsPacket).bc(7).i32(100).i32(200);
            assert_eq!(ServerMsg::decode_batch(&batch.finish(), 2), vec![expected, ServerMsg::UnitPoints { obj_id: 7, hp: 1, mp: 2 }]);
            for len in 0..body.len() {
                let mut frame = Writer::new();
                frame.u8(MAGIC).u8(LEVEL_GAME).u8(0).u8(0).u16(sc::SCNotifyResurrectionPacket).raw(&body[..len]);
                assert!(ServerMsg::decode(&frame.finish()).is_err(), "kind {kind} length {len}");
            }
        }
    }

    #[test]
    fn progression_packets_golden_bytes() {
        // SCExpChanged(objId, exp, shouldAddAbilityExp): bc, i32, bool.
        let mut w = Writer::new();
        w.bc(0x112233).i32(-250).bool(true);
        assert_eq!(
            decode(sc::SCExpChangedPacket, &w.finish()),
            ServerMsg::ExpChanged { obj_id: 0x112233, delta: -250, add_ability_exp: true }
        );
        // SCAbilityExpChanged(objId, ability, exp)
        let mut w = Writer::new();
        w.bc(7).u8(3).i32(12_345);
        assert_eq!(
            decode(sc::SCAbilityExpChangedPacket, &w.finish()),
            ServerMsg::AbilityExpChanged { obj_id: 7, ability: 3, exp: 12_345 }
        );
        // SCRecoverableExp(objId, recoverable, penaltied, reason)
        let mut w = Writer::new();
        w.bc(7).i32(100).i32(20).i32(1);
        assert_eq!(
            decode(sc::SCRecoverableExpPacket, &w.finish()),
            ServerMsg::RecoverableExp { obj_id: 7, recoverable: 100, penaltied: 20, reason: 1 }
        );
        // SCLevelChanged(objId, level): bc, u8
        assert_eq!(
            decode(sc::SCLevelChangedPacket, &[7, 0, 0, 12]),
            ServerMsg::LevelChanged { obj_id: 7, level: 12 }
        );
        // SCAbilitySwapped(objId, old, new)
        assert_eq!(
            decode(sc::SCAbilitySwappedPacket, &[7, 0, 0, 1, 4]),
            ServerMsg::AbilitySwapped { obj_id: 7, old_ability: 1, ability: 4 }
        );
        // SCCharacterLaborPowerChanged(amount, action, point, step): i32, i32, i32, u8
        let mut w = Writer::new();
        w.i32(10).i32(0).i32(0).u8(0);
        assert_eq!(
            decode(sc::SCCharacterLaborPowerChangedPacket, &w.finish()),
            ServerMsg::LaborPowerChanged { amount: 10, action: 0, point: 0, step: 0 }
        );
        let mut w = Writer::new();
        w.i32(-30).i32(5).i32(2).u8(1);
        assert_eq!(
            decode(sc::SCCharacterLaborPowerChangedPacket, &w.finish()),
            ServerMsg::LaborPowerChanged { amount: -30, action: 5, point: 2, step: 1 }
        );
        // SCCharacterGamePoints: honor, vocation, eight more i32.
        let mut w = Writer::new();
        w.i32(11).i32(22);
        for i in 0..8 {
            w.i32(i);
        }
        let ServerMsg::GamePoints { honor, vocation, other } = decode(sc::SCCharacterGamePointsPacket, &w.finish())
        else {
            panic!("not game points")
        };
        assert_eq!((honor, vocation, other[7]), (11, 22, 7));
        // SCGamePointChanged(kind, amount): u8, i32
        let mut w = Writer::new();
        w.u8(1).i32(-4);
        assert_eq!(
            decode(sc::SCGamePointChangedPacket, &w.finish()),
            ServerMsg::GamePointChanged { kind: 1, amount: -4 }
        );
        // SCBmPoint(bmPoint): i64
        assert_eq!(decode(sc::SCBmPointPacket, &9_000_000_000i64.to_le_bytes()), ServerMsg::BmPoint(9_000_000_000));
        // SCICSCashPoint(point): i32 (the lane sends it 1.4 s after the login; it used to arrive as an unhandled packet)
        assert_eq!(decode(sc::SCICSCashPointPacket, &1500i32.to_le_bytes()), ServerMsg::CashPoint(1500));
        assert_eq!(decode(sc::SCCombatEngagedPacket, &[5, 0, 0]), ServerMsg::CombatEngaged { obj_id: 5 });
        assert_eq!(decode(sc::SCCombatClearedPacket, &[5, 0, 0]), ServerMsg::CombatCleared { obj_id: 5 });
    }

    #[test]
    fn action_bar_and_bag_frames() {
        use crate::items::{container, ActionKind};
        // SCActionSlots: a skill, an empty slot, an item type.
        let mut w = Writer::new();
        w.u8(2).u32(18132).u8(0).u8(1).u32(8001);
        let ServerMsg::ActionSlots(slots) = decode(sc::SCActionSlotsPacket, &w.finish()) else { panic!() };
        assert_eq!(slots.len(), 3);
        assert_eq!((slots[0].kind, slots[0].id), (ActionKind::Spell, 18132));
        assert_eq!(slots[2].item_template_id(), Some(8001));
        // SCCharacterInvenInit(numInvenSlots, numBankSlots): two u32.
        let mut w = Writer::new();
        w.u32(50).u32(200);
        assert_eq!(
            decode(sc::SCCharacterInvenInitPacket, &w.finish()),
            ServerMsg::InvenInit { bag_slots: 50, bank_slots: 200 }
        );
        // SCCharacterInvenContents: an empty chunk is the type, 1 chunk, index and ten zero ids.
        let mut w = Writer::new();
        w.u8(container::INVENTORY).u8(1).u8(4);
        for _ in 0..10 {
            w.i32(0);
        }
        let body = w.finish();
        assert_eq!(body.len(), 43);
        let ServerMsg::InvenContents(c) = decode(sc::SCCharacterInvenContentsPacket, &body) else { panic!() };
        assert_eq!((c.container, c.start_chunk, c.items.len(), c.first_slot()), (2, 4, 10, 40));
        // SCInvenExpanded(slotType, numSlots)
        assert_eq!(
            decode(sc::SCInvenExpandedPacket, &[2, 60]),
            ServerMsg::InvenExpanded { container: 2, slots: 60 }
        );
        // SCUnitEquipmentsChanged(objId, count, (slot, item)...): one worn item, one emptied slot.
        let mut w = Writer::new();
        w.bc(0x112233).u8(2);
        w.u8(15).u32(5569).u64(900).u8(1).u8(0).i32(1).u8(0).i64(0).i32(0).u32(0).u8(1).i64(0).i64(0);
        w.u8(16).i32(0);
        let ServerMsg::EquipmentChanged { obj_id, items } = decode(sc::SCUnitEquipmentsChangedPacket, &w.finish())
        else {
            panic!()
        };
        assert_eq!(obj_id, 0x112233);
        assert_eq!(items[0].0, 15);
        assert_eq!(items[0].1.as_ref().unwrap().template_id, 5569);
        assert_eq!(items[1], (16, None));
        // SCItemTaskSuccess with a money task, as the server's own test builds it
        // (`[73, 1, 1, i32, 0, 0 force remove, u32 type]`).
        let body = [73, 1, 1, 0xF0, 0xD8, 0xFF, 0xFF, 0, 0, 0, 0, 0];
        let ServerMsg::ItemTasks(t) = decode(sc::SCItemTaskSuccessPacket, &body) else { panic!() };
        assert_eq!(t.reason, 73);
        assert_eq!(t.tasks, vec![crate::items::ItemTask::Money(-10_000)]);
    }

    #[test]
    fn execute_craft_is_one_native_batch_with_an_exact_eleven_byte_body() {
        let cases = [
            (1, 1, 1, vec![1, 0, 0, 0, 1, 0, 0, 1, 0, 0, 0]),
            (
                0x11223344,
                0x445566,
                3,
                vec![0x44, 0x33, 0x22, 0x11, 0x66, 0x55, 0x44, 3, 0, 0, 0],
            ),
            (
                u32::MAX,
                0xffffff,
                i32::MAX,
                vec![255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 127],
            ),
        ];
        for (craft_id, doodad_obj_id, count, expected) in cases {
            let message = ClientMsg::ExecuteCraft {
                craft_id,
                doodad_obj_id,
                count,
            };
            assert_eq!(message.opcode(), 0xf8);
            assert_eq!(message.name(), "CSExecuteCraft");
            assert_eq!(message.encode_body(), expected);
            let mut frame = vec![0xdd, 1, 0, 9, 0xf8, 0];
            frame.extend_from_slice(&expected);
            assert_eq!(message.encode(9), frame);
            // Consume exactly what the lane's CSExecuteCraft reader consumes.
            let mut reader = Reader::new(&expected);
            assert_eq!(
                (
                    reader.u32().unwrap(),
                    reader.bc().unwrap(),
                    reader.i32().unwrap()
                ),
                (craft_id, doodad_obj_id, count)
            );
            assert_eq!(reader.remaining(), 0);
        }
    }

    #[test]
    fn client_action_slot_update_bytes() {
        use crate::items::{ActionKind, ActionSlot};
        // CSUpdateActionSlot: slot, kind, id (u32 for spells and item types, u64 for item ids).
        let spell = ClientMsg::UpdateActionSlot { slot: 4, action: ActionSlot { kind: ActionKind::Spell, id: 18132 } };
        assert_eq!(spell.encode_body(), [4, 2, 0xD4, 0x46, 0, 0]);
        let item = ClientMsg::UpdateActionSlot { slot: 0, action: ActionSlot { kind: ActionKind::ItemId, id: 258 } };
        assert_eq!(item.encode_body(), [0, 4, 2, 1, 0, 0, 0, 0, 0, 0]);
        assert_eq!(ClientMsg::UpdateActionSlot { slot: 7, action: ActionSlot::default() }.encode_body(), [7, 0]);
        assert_eq!(spell.opcode(), 0x0b6);
    }

    #[test]
    fn client_item_move_bytes() {
        // CSSwapItems: from item id, to item id (u64 each), then from container, slot, to container, slot. Equipping into
        // an empty worn slot is "from the empty equipment slot (item 0) to the bag item".
        let equip = ItemMove { from_item_id: 0, to_item_id: 0x0102, from_container: 1, from_slot: 15, to_container: 2, to_slot: 3 };
        let swap = ClientMsg::SwapItems(equip);
        assert_eq!(swap.encode_body(), [0, 0, 0, 0, 0, 0, 0, 0, 2, 1, 0, 0, 0, 0, 0, 0, 1, 15, 2, 3]);
        assert_eq!(swap.opcode(), 0x03a);
        // CSSplitBagItem: the same block and a count (i32).
        let split = ClientMsg::SplitItem { mv: equip, count: 2 };
        assert_eq!(&split.encode_body()[..20], &swap.encode_body()[..]);
        assert_eq!(&split.encode_body()[20..], [2, 0, 0, 0]);
        assert_eq!(split.opcode(), 0x039);
        // CSDestroyItem: item id (u64), container, slot, count (i32).
        let destroy = ClientMsg::DestroyItem { item_id: 7, container: 2, slot: 9, count: 3 };
        assert_eq!(destroy.encode_body(), [7, 0, 0, 0, 0, 0, 0, 0, 2, 9, 3, 0, 0, 0]);
        assert_eq!(destroy.opcode(), 0x038);
    }

    #[test]
    fn unit_death_with_and_without_a_killer() {
        // SCUnitDeath: bc obj, kill reason, resurrect wait, lost exp, durability loss, bc killer
        // (0 = none) and for a killer game type, kill streak, three flags and the name.
        let mut w = Writer::new();
        w.bc(0x10).u8(1).i32(10_000).i32(-77).u8(5).bc(0);
        assert_eq!(
            decode(sc::SCUnitDeathPacket, &w.finish()),
            ServerMsg::UnitDeath {
                obj_id: 0x10,
                kill_reason: 1,
                resurrect_wait_ms: 10_000,
                lost_exp: -77,
                durability_loss_rate: 5,
                killer: None
            }
        );
        let mut w = Writer::new();
        w.bc(0x10).u8(0).i32(0).i32(0).u8(0).bc(0x20);
        w.u8(0).u16(0).u8(0).u8(0).u8(0).string("Slayer");
        let ServerMsg::UnitDeath { killer, .. } = decode(sc::SCUnitDeathPacket, &w.finish()) else { panic!() };
        assert_eq!(killer, Some((0x20, "Slayer".to_string())));
        // SCCharacterResurrected(unitId, x, y, z, zRot)
        let mut w = Writer::new();
        w.bc(0x10).position(Pos::new(100.0, 200.0, 30.0)).f32(1.5);
        let ServerMsg::CharacterResurrected { obj_id, pos, yaw } = decode(sc::SCCharacterResurrectedPacket, &w.finish())
        else {
            panic!()
        };
        assert_eq!(obj_id, 0x10);
        assert!((pos.x - 100.0).abs() < 0.01 && (pos.y - 200.0).abs() < 0.01 && (yaw - 1.5).abs() < 1e-6);
    }

    #[test]
    fn buff_and_faction_packets_golden_bytes() {
        // SCBuffRemoved(objId, index): bc, u32; SCBuffLearned(objId, buffId): bc, u32.
        let mut w = Writer::new();
        w.bc(0x0A0B0C).u32(7);
        let body = w.finish();
        assert_eq!(decode(sc::SCBuffRemovedPacket, &body), ServerMsg::BuffRemoved { obj_id: 0x0A0B0C, index: 7 });
        assert_eq!(decode(sc::SCBuffLearnedPacket, &body), ServerMsg::BuffLearned { obj_id: 0x0A0B0C, buff_id: 7 });
        // SCBuffCreated: see `buff::tests::buff_created_golden_bytes` for the field walk.
        let mut w = Writer::new();
        w.u8(0).bc(0x10); // caster
        w.u32(0).bc(0x20).u32(4).u32(2423).u8(1).i16(1).u32(0);
        w.pisc(&[0, 100, 0, 0]);
        let ServerMsg::BuffCreated { obj_id, buff } = decode(sc::SCBuffCreatedPacket, &w.finish()) else { panic!() };
        assert_eq!((obj_id, buff.index, buff.buff_id, buff.duration_ms), (0x20, 4, 2423, 1000));
        // SCUnitFactionChanged(unitId, unitName, id, id2, temp), as in the server's tests.
        let body = [0x10, 0, 0, 1, 0, b'A', 101, 0, 0, 0, 161, 0, 0, 0, 0];
        assert_eq!(
            decode(sc::SCUnitFactionChangedPacket, &body),
            ServerMsg::Expedition(crate::ExpeditionEvent::UnitFactionChanged {
                obj_id:0x10,name:vec![b'A'],old_faction_id:101,new_faction_id:161,temporary:0,
            })
        );
    }

    #[test]
    fn world_packets_golden_bytes() {
        // SCTimeOfDay: f32 hour. SCDetailedTimeOfDay: hour, speed 0.0016666, start 0, end 24.
        let mut w = Writer::new();
        w.f32(13.5);
        assert_eq!(
            decode(sc::SCTimeOfDayPacket, &w.finish()),
            ServerMsg::TimeOfDay { hour: 13.5, speed: None }
        );
        let mut w = Writer::new();
        w.f32(6.25).f32(0.001_666_6).f32(0.0).f32(24.0);
        let bytes = w.finish();
        assert_eq!(bytes.len(), 16);
        assert_eq!(
            decode(sc::SCDetailedTimeOfDayPacket, &bytes),
            ServerMsg::TimeOfDay { hour: 6.25, speed: Some(0.001_666_6) }
        );
        // SCOnOffSnow: bool.
        assert_eq!(decode(sc::SCOnOffSnowPacket, &[1]), ServerMsg::Snow(true));
        assert_eq!(decode(sc::SCOnOffSnowPacket, &[0]), ServerMsg::Snow(false));
        // SCJoinedChatChannel: i16 type, i16 sub type (zone group), u32 faction, string name.
        // "Entering Chat: 1.Shout.Solzreed Peninsula" is Shout (1) in zone group 5.
        let mut w = Writer::new();
        w.i16(1).i16(5).u32(0).string("");
        let bytes = w.finish();
        assert_eq!(bytes, [1, 0, 5, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(
            decode(sc::SCJoinedChatChannelPacket, &bytes),
            ServerMsg::ChatJoined { chat_type: 1, sub_type: 5, faction_id: 0, name: String::new() }
        );
        // SCLeavedChatChannel: the same without the name.
        assert_eq!(
            decode(sc::SCLeavedChatChannelPacket, &[1, 0, 5, 0, 0, 0, 0, 0]),
            ServerMsg::ChatLeft { chat_type: 1, sub_type: 5, faction_id: 0 }
        );
        // SCTargetChanged: bc object id, bc target (0 = none).
        let mut w = Writer::new();
        w.bc(0x0102).bc(0x0A0B0C);
        let bytes = w.finish();
        assert_eq!(bytes.len(), 6);
        assert_eq!(
            decode(sc::SCTargetChangedPacket, &bytes),
            ServerMsg::TargetChanged { obj_id: 0x0102, target_obj_id: 0x0A0B0C }
        );
        // CSChangeTarget: the bc target.
        assert_eq!(ClientMsg::ChangeTarget { obj_id: 0x0A0B0C }.encode_body().len(), 3);
    }

    /// A stored-block deflate stream as the server writes it (`deflate_stored`).
    fn stored(data: &[u8], last: bool) -> Vec<u8> {
        let len = data.len() as u16;
        let mut out = vec![u8::from(last)];
        out.extend(len.to_le_bytes());
        out.extend((!len).to_le_bytes());
        out.extend(data);
        out
    }

    #[test]
    fn quest_packets_decode() {
        // SCQuestContextCompleted(questId, 8 flag bytes, componentId): quest 330 is bit 10 of block 5
        let mut w = Writer::new();
        w.u32(330).raw(&[0, 0b0000_0100, 0, 0, 0, 0, 0, 0]).u32(0);
        match decode(sc::SCQuestContextCompletedPacket, &w.finish()) {
            ServerMsg::QuestCompleted { quest_id: 330, block } => assert!(block.id == 5 && block.has(330)),
            other => panic!("{other:?}"),
        }
        // SCCompletedQuests(count, (u16 block id, 8 bytes)...)
        let mut w = Writer::new();
        w.i32(1).u16(0).raw(&[0b10, 0, 0, 0, 0, 0, 0, 0]);
        match decode(sc::SCCompletedQuestsPacket, &w.finish()) {
            ServerMsg::CompletedQuests(blocks) => assert!(blocks.len() == 1 && blocks[0].has(1) && !blocks[0].has(0)),
            other => panic!("{other:?}"),
        }
        // SCQuests with none
        assert_eq!(decode(sc::SCQuestsPacket, &[0, 0, 0, 0]), ServerMsg::Quests(vec![]));
    }

    #[test]
    fn skill_packets_golden_bytes() {
        // SCSkillEnded(tl), SCCastingStopped(tl, duration), SCSkillStopped(obj, skill).
        assert_eq!(decode(sc::SCSkillEndedPacket, &[7, 0]), ServerMsg::SkillEnded { tl: 7 });
        assert_eq!(
            decode(sc::SCCastingStoppedPacket, &[7, 0, 0xE8, 0x03, 0, 0]),
            ServerMsg::CastingStopped { tl: 7, duration_ms: 1000 }
        );
        let mut w = Writer::new();
        w.bc(0x0102).u32(18132);
        assert_eq!(decode(sc::SCSkillStoppedPacket, &w.finish()), ServerMsg::SkillStopped { obj_id: 0x0102, skill_id: 18132 });
        // SCSkillCooldownReset(obj, skill, tag, gcd).
        let mut w = Writer::new();
        w.bc(5).u32(18132).u32(77).bool(true);
        assert_eq!(
            decode(sc::SCSkillCooldownResetPacket, &w.finish()),
            ServerMsg::SkillCooldownReset { obj_id: 5, skill_id: 18132, tag_id: 77, gcd: true }
        );
        // SCSkillLearned / SCSkillUpgraded(id, level), SCSkillsReset(obj, ability).
        let mut w = Writer::new();
        w.u32(16064).u8(2);
        assert_eq!(decode(sc::SCSkillLearnedPacket, &w.finish()), ServerMsg::SkillLearned { skill_id: 16064, level: 2 });
        let mut w = Writer::new();
        w.u32(16064).u8(3);
        assert_eq!(decode(sc::SCSkillUpgradedPacket, &w.finish()), ServerMsg::SkillUpgraded { skill_id: 16064, level: 3 });
        let mut w = Writer::new();
        w.bc(5).u8(4);
        assert_eq!(decode(sc::SCSkillsResetPacket, &w.finish()), ServerMsg::SkillsReset { obj_id: 5, ability: 4 });
        // SCCombatText(source, target, type).
        let mut w = Writer::new();
        w.bc(5).bc(6).u8(2);
        assert_eq!(decode(sc::SCCombatTextPacket, &w.finish()), ServerMsg::CombatText { source: 5, target: 6, text_type: 2 });
        // SCPlotEnded(tl), SCPlotCastingStopped / SCPlotChannelingStopped(tl, duration, last event).
        assert_eq!(decode(sc::SCPlotEndedPacket, &[9, 0]), ServerMsg::PlotEnded { tl: 9 });
        let body = [9, 0, 100, 0, 0, 0, 3];
        assert_eq!(
            decode(sc::SCPlotCastingStoppedPacket, &body),
            ServerMsg::PlotStopped { tl: 9, duration_ms: 100, last_event: 3, channeling: false }
        );
        assert_eq!(
            decode(sc::SCPlotChannelingStoppedPacket, &body),
            ServerMsg::PlotStopped { tl: 9, duration_ms: 100, last_event: 3, channeling: true }
        );
        // CSStartSkill from the client: skill, unit caster, unit target, no skill object.
        let m = ClientMsg::StartSkill {
            skill_id: 18132,
            self_obj_id: 0x0102,
            caster: StartCaster::Unit,
            target: SkillTarget::unit(0x0A0B0C),
        };
        assert_eq!(m.encode_body(), [0xD4, 0x46, 0, 0, 0, 0x02, 0x01, 0, 0, 0x0C, 0x0B, 0x0A, 0]);
        assert_eq!(m.encode(1)[..6], [0xDD, 0x01, 0, 1, 0x52, 0x00]);
        // CSStopCasting: timeline, plot timeline, the unit.
        let stop = ClientMsg::StopCasting { tl: 7, plot_tl: 0, obj_id: 0x0102 };
        assert_eq!(stop.encode_body(), [7, 0, 0, 0, 0x02, 0x01, 0]);
        assert_eq!(stop.encode(3)[..6], [0xDD, 0x01, 0, 3, 0x54, 0x00]);
        assert_eq!(ClientMsg::StopCasting { tl: 0, plot_tl: 0x0A0B, obj_id: 1 }.encode_body(), [0, 0, 0x0B, 0x0A, 1, 0, 0]);
        // The loot packets, byte for byte as the server's own tests build them (bc = 3 bytes, then a pad byte).
        assert_eq!(ClientMsg::LootOpenBag { obj_id: 0x0102, loot_all: true }.encode_body(), [2, 1, 0, 0, 0, 0, 1]);
        assert_eq!(ClientMsg::LootItem { item_index: 2, owner_type: 1, owner_obj_id: 0x10 }.encode_body(), [2, 0, 1, 0, 0x10, 0, 0, 0, 0, 0, 0]);
        assert_eq!(ClientMsg::LootCloseBag { owner_type: 1, owner_obj_id: 0x10 }.encode_body(), [0, 0, 1, 0, 0x10, 0, 0, 0]);
        assert_eq!(ClientMsg::LootOpenBag { obj_id: 1, loot_all: false }.opcode(), 0x08e);
        assert_eq!(decode(sc::SCLootableStatePacket, &[0, 0, 1, 0, 4, 3, 2, 0, 1]), ServerMsg::Lootable { owner_type: 1, obj_id: 0x020304, has_loot: true });
        assert_eq!(decode(sc::SCUnitLootingStatePacket, &[0x20, 0, 0, 1]), ServerMsg::UnitLooting { obj_id: 0x20, looting: 1 });
        assert_eq!(
            decode(sc::SCLootItemTookPacket, &[4, 3, 2, 1, 2, 0, 1, 0, 0xef, 0xcd, 0xab, 0, 5, 0, 0, 0]),
            ServerMsg::LootItemTook { template_id: 0x0102_0304, item_index: 2, owner_type: 1, owner_obj_id: 0x00ab_cdef, count: 5 }
        );
        assert_eq!(
            decode(sc::SCLootItemFailedPacket, &[25, 0, 0, 0, 3, 0, 1, 0, 0x10, 0, 0, 0, 9, 0, 0, 0]),
            ServerMsg::LootItemFailed { error: 25, item_index: 3, owner_type: 1, owner_obj_id: 0x10, template_id: 9 }
        );
        // SCLootBagData: a count, the items as the bag lists them, "loot all"
        let mut w = Writer::new();
        w.u8(1).u32(77).u64((0x20u64 << 32) + (1 << 16) + 2).u8(1).u8(0).i32(10).u8(0).i64(0).i32(0).u32(0).u8(0).i64(0).i64(0).bool(true);
        let ServerMsg::LootBag { items, loot_all } = decode(sc::SCLootBagDataPacket, &w.finish()) else { panic!() };
        assert!(loot_all && items.len() == 1);
        assert_eq!((items[0].template_id, items[0].count, items[0].grade), (77, 10, 1));
        assert_eq!(crate::state::item_index(&items[0]), 2);
        // CSResurrectCharacter: one bool, in place or at the return point.
        assert_eq!(ClientMsg::Resurrect { in_place: true }.encode(2), [0xDD, 0x01, 0, 2, 0x4E, 0x00, 1]);
        assert_eq!(ClientMsg::Resurrect { in_place: false }.encode_body(), [0]);
        // The skill book: CSLearnSkill / CSLearnBuff (one id), CSResetSkills (ability, ausp), CSSwapAbility (unit, old, new, auap).
        assert_eq!(ClientMsg::LearnSkill { skill_id: 0x0102_0304 }.encode(4), [0xDD, 0x01, 0, 4, 0x92, 0x00, 4, 3, 2, 1]);
        assert_eq!(ClientMsg::LearnBuff { passive_id: 7 }.encode_body(), [7, 0, 0, 0]);
        assert_eq!(ClientMsg::ResetSkills { ability: 6 }.encode_body(), [6, 0]);
        assert_eq!(ClientMsg::ResetSkills { ability: 6 }.opcode(), 0x094);
        assert_eq!(ClientMsg::SwapAbility { obj_id: 0x0102, old: 11, ability: 6 }.encode_body(), [2, 1, 0, 11, 6, 0]);
        assert_eq!(ClientMsg::SwapAbility { obj_id: 1, old: 11, ability: 6 }.opcode(), 0x096);
    }

    #[test]
    fn a_batch_resumes_after_a_malformed_packet() {
        fn batch(raw: &[u8], count: u16) -> Vec<ServerMsg> {
            let mut frame = vec![MAGIC, LEVEL_COMPRESSED];
            frame.extend(count.to_le_bytes());
            frame.extend(stored(raw, true));
            let ServerMsg::Batch(list) = ServerMsg::decode(&frame).unwrap() else { panic!("not a batch") };
            list
        }
        // An unknown packet whose body holds a false packet start (`00 00` + the snow opcode + 1, but
        // not followed by another entry), between two good packets: the resync skips the false start.
        let mut raw = Writer::new();
        raw.u16(0).u16(sc::SCSkillEndedPacket).u16(1);
        raw.u16(0).u16(0x7fff).raw(&[0, 0]).u16(sc::SCOnOffSnowPacket).raw(&[1, 5]);
        raw.u16(0).u16(sc::SCSkillEndedPacket).u16(2);
        let list = batch(&raw.finish(), 3);
        assert_eq!(
            list,
            [
                ServerMsg::SkillEnded { tl: 1 },
                ServerMsg::Unhandled { level: LEVEL_COMPRESSED, opcode: 0x7fff, len: 6 },
                ServerMsg::SkillEnded { tl: 2 }
            ]
        );

        // A count that does not match what follows: no resync point is trusted, the batch ends.
        let mut raw = Writer::new();
        raw.u16(0).u16(0x7fff).raw(&[9, 9]);
        raw.u16(0).u16(sc::SCSkillEndedPacket).u16(3);
        let list = batch(&raw.finish(), 5);
        assert_eq!(list.len(), 1);
        assert!(matches!(list[0], ServerMsg::Unhandled { opcode: 0x7fff, .. }));
    }

    #[test]
    fn compressed_batches_are_unpacked() {
        // CompressedGamePackets: [dd][04][u16 count][deflate([u16 0][u16 opcode][body] ...)].
        let mut raw = Writer::new();
        raw.u16(0).u16(sc::SCSkillEndedPacket).u16(7);
        raw.u16(0).u16(sc::SCSkillCooldownResetPacket).bc(5).u32(18132).u32(0).bool(false);
        // SCUnitHealed: cast action (skill 25202, tl 9), unit caster, bc target, type, hit, value, o, result.
        raw.u16(0).u16(sc::SCUnitHealedPacket);
        raw.u8(0).u32(25202).u16(9).u8(0).bc(5).bc(5).u8(0).u8(13).i32(300).i32(0).u8(1);
        let raw = raw.finish();
        let mut frame = vec![MAGIC, LEVEL_COMPRESSED, 3, 0];
        frame.extend(stored(&raw, true));
        let ServerMsg::Batch(list) = ServerMsg::decode(&frame).unwrap() else { panic!("not a batch") };
        assert_eq!(list.len(), 3);
        assert_eq!(list[0], ServerMsg::SkillEnded { tl: 7 });
        assert!(matches!(list[1], ServerMsg::SkillCooldownReset { skill_id: 18132, .. }));
        let ServerMsg::UnitHealed(h) = &list[2] else { panic!("not a heal") };
        assert_eq!((h.target_obj_id, h.value), (5, 300));
        assert_eq!(ServerMsg::Batch(list).name(), "CompressedGamePackets");

        // The stream may span several stored blocks; a packet we cannot decode is skipped: the batch
        // resumes at the next packet start from which the rest decodes to the end.
        let mut raw = Writer::new();
        raw.u16(0).u16(sc::SCSkillEndedPacket).u16(8);
        raw.u16(0).u16(0x7fff).raw(&[1, 2, 3]);
        raw.u16(0).u16(sc::SCSkillEndedPacket).u16(9);
        let raw = raw.finish();
        let (a, b) = raw.split_at(5);
        let mut frame = vec![MAGIC, LEVEL_COMPRESSED, 3, 0];
        frame.extend(stored(a, false));
        frame.extend(stored(b, true));
        let ServerMsg::Batch(list) = ServerMsg::decode(&frame).unwrap() else { panic!("not a batch") };
        assert_eq!(list.len(), 3);
        assert_eq!(list[0], ServerMsg::SkillEnded { tl: 8 });
        assert_eq!(list[1], ServerMsg::Unhandled { level: LEVEL_COMPRESSED, opcode: 0x7fff, len: 3 });
        assert_eq!(list[2], ServerMsg::SkillEnded { tl: 9 });

        // Truncated Huffman/stored blocks are invalid, rather than silently skipped batches.
        assert!(ServerMsg::decode(&[MAGIC, LEVEL_COMPRESSED, 2, 0, 0x4B, 0x2C, 0x00]).is_err());
        assert_eq!(crate::compression::inflate_batch(&stored(&[1, 2, 3], true)).unwrap(), vec![1, 2, 3]);
        assert!(crate::compression::inflate_batch(&[1, 3, 0, 0, 0, 1, 2, 3]).is_err(), "length check");
    }

    #[test]
    fn doodad_phase_full_nineteen_bytes_keep_the_next_dd04_packet_aligned() {
        // Lane doodad/packets.rs::sc_doodad_phase_changed golden: growing30s, puzzle=-1.
        let body = [
            3, 2, 1, 0x2c, 0x12, 0, 0, 0x30, 0x75, 0, 0, 255, 255, 255, 255, 0xd6, 0x6b, 0, 0,
        ];
        let expected = ServerMsg::DoodadPhaseChanged {
            obj_id: 0x010203,
            func_group_id: 4652,
            time_left: 30000,
            puzzle_group: -1,
            item_template_id: 27606,
        };
        assert_eq!(
            ServerMsg::decode(&game_frame(sc::SCDoodadPhaseChangedPacket, &body)).unwrap(),
            expected
        );
        for cut in 0..body.len() {
            assert!(
                ServerMsg::decode(&game_frame(sc::SCDoodadPhaseChangedPacket, &body[..cut])).is_err(),
                "cut{cut}"
            );
        }
        let mut raw = Writer::new();
        raw.u16(0)
            .u16(sc::SCDoodadPhaseChangedPacket)
            .raw(&body)
            .u16(0)
            .u16(sc::SCOnOffSnowPacket)
            .bool(true);
        let mut frame = vec![MAGIC, LEVEL_COMPRESSED, 2, 0];
        frame.extend(stored(&raw.finish(), true));
        assert_eq!(
            ServerMsg::decode(&frame).unwrap(),
            ServerMsg::Batch(vec![expected, ServerMsg::Snow(true)])
        );
    }

    #[test]
    fn doodad_mutation_packets_match_lane_goldens_and_keep_following_dd04_boundaries() {
        let packets = vec![
            (
                sc::SCDoodadChangedPacket,
                vec![8, 0, 0, 253, 255, 255, 255],
                ServerMsg::DoodadChanged {
                    obj_id: 8,
                    data: -3,
                },
            ),
            (
                sc::SCDoodadOriginatorPacket,
                vec![9, 0, 0, 10, 0, 0, 0, 148, 0, 0, 0],
                ServerMsg::DoodadOriginator {
                    obj_id: 9,
                    new_owner_id: 10,
                    new_faction: 148,
                },
            ),
            (
                sc::SCDoodadSoundPacket,
                vec![7, 0, 0, 55, 0, 0, 0],
                ServerMsg::DoodadSound {
                    obj_id: 7,
                    sound_id: 55,
                },
            ),
            (
                sc::SCVegetationCutdowningPacket,
                vec![5, 0, 0, 6, 0, 0],
                ServerMsg::VegetationCutdown {
                    unit_obj_id: 5,
                    doodad_obj_id: 6,
                },
            ),
        ];
        let mut raw = Writer::new();
        let mut expected = Vec::new();
        for (op, body, message) in packets {
            assert_eq!(ServerMsg::decode(&game_frame(op, &body)).unwrap(), message);
            for cut in 0..body.len() {
                assert!(
                    ServerMsg::decode(&game_frame(op, &body[..cut])).is_err(),
                    "op{op:03x}cut{cut}"
                );
            }
            raw.u16(0).u16(op).raw(&body);
            expected.push(message);
        }
        raw.u16(0)
            .u16(sc::SCDoodadPhaseChangedPacket)
            .bc(9)
            .u32(4652)
            .u32(30000)
            .i32(-1)
            .u32(27606);
        expected.push(ServerMsg::DoodadPhaseChanged {
            obj_id: 9,
            func_group_id: 4652,
            time_left: 30000,
            puzzle_group: -1,
            item_template_id: 27606,
        });
        raw.u16(0).u16(sc::SCOnOffSnowPacket).bool(true);
        expected.push(ServerMsg::Snow(true));
        let mut frame = vec![MAGIC, LEVEL_COMPRESSED, 5, 0];
        frame.extend(stored(&raw.finish(), true));
        assert_eq!(
            ServerMsg::decode(&frame).unwrap(),
            ServerMsg::Batch(expected)
        );
        let mut owner = Writer::new();
        owner.bc(9).u32(0xf1234567).u32(0x80000000);
        assert_eq!(
            decode(sc::SCDoodadOriginatorPacket, &owner.finish()),
            ServerMsg::DoodadOriginator {
                obj_id: 9,
                new_owner_id: 0xf1234567,
                new_faction: 0x80000000
            }
        );
    }

    #[test]
    fn a_combat_batch_reports_fire_hit_and_death_in_order() {
        // What the lane server sends for the killing arrow of Shoot Arrow, in one DD04 batch: the skill fires
        // (target a unit, fire anim 32, effects land 370 ms later), the hit, the death with its killer, the end.
        let mut raw = Writer::new();
        raw.u16(0).u16(sc::SCSkillFiredPacket);
        raw.u32(16064).u16(1291).u8(0).bc(0x10).u8(0).bc(0x20).u8(0);
        raw.i16(37 + 10).i16(10).u8(0).u32(32).u8(0x82);
        raw.u16(0).u16(sc::SCUnitDamagedPacket);
        raw.u8(0).u32(16064).u16(1291).u8(0).bc(0x10).bc(0x10).bc(0x20).u8(0);
        raw.pisc(&[24, 0, 0]).pisc(&[0, 0, 0]).u8(0).u16(288 | 0x09).u8(1).u8(1);
        raw.u16(0).u16(sc::SCUnitDeathPacket).bc(0x20).u8(1).i32(10).i32(0).u8(0).bc(0x10);
        raw.u8(0).u16(0).u8(0).u8(0).u8(0).string("Bevytest");
        raw.u16(0).u16(sc::SCSkillEndedPacket).u16(1291);
        let raw = raw.finish();
        let mut frame = vec![MAGIC, LEVEL_COMPRESSED, 4, 0];
        frame.extend(stored(&raw, true));
        let ServerMsg::Batch(list) = ServerMsg::decode(&frame).unwrap() else { panic!("not a batch") };
        assert_eq!(list.len(), 4, "{list:?}");
        let ServerMsg::SkillFired(f) = &list[0] else { panic!("not a fire: {:?}", list[0]) };
        assert_eq!((f.skill_id, f.tl, f.caster.obj_id, f.target.obj_id, f.delay_ms, f.fire_anim_id), (16064, 1291, 0x10, 0x20, 370, 32));
        assert_eq!(f.raw_final_flag, 0x82);
        let ServerMsg::UnitDamaged(d) = &list[1] else { panic!("not a hit: {:?}", list[1]) };
        assert_eq!((d.caster_obj_id, d.target_obj_id, d.damage, d.action.tl), (0x10, 0x20, 24, 1291));
        assert_eq!(d.hit_type, crate::skill::hit_type::RANGED_HIT);
        assert_eq!((d.holdable_id, d.raw_hit, d.pierce_selector()), (0, 0x0129, 0));
        assert!(matches!(&list[2], ServerMsg::UnitDeath { obj_id: 0x20, killer: Some((0x10, name)), .. } if name == "Bevytest"));
        assert_eq!(list[3], ServerMsg::SkillEnded { tl: 1291 });
    }

    #[test]
    fn cancel_delete_response_golden_bytes() {
        // SCCancelCharacterDeleteResponsePacket(characterId, status) as the server writes it: u32 id,
        // u8 status (3 restored, 4 unknown character); body [9, 0, 0, 0, 3] in the server's
        // `world_flow_tests`. Frame: [dd][01][hash 00][count 00][u16 opcode 0x3c][body].
        let frame = [0xDD, 0x01, 0x00, 0x00, 0x3C, 0x00, 9, 0, 0, 0, 3];
        assert_eq!(
            ServerMsg::decode(&frame).unwrap(),
            ServerMsg::CancelCharacterDeleteResponse { id: 9, status: 3 }
        );
        let frame = [0xDD, 0x01, 0x00, 0x00, 0x3C, 0x00, 77, 0, 0, 0, 4];
        assert_eq!(
            ServerMsg::decode(&frame).unwrap(),
            ServerMsg::CancelCharacterDeleteResponse { id: 77, status: 4 }
        );
        // CSCancelCharacterDelete from the client: the u32 id only.
        assert_eq!(ClientMsg::CancelDeleteCharacter { id: 258 }.encode_body(), [2, 1, 0, 0]);
    }

    #[test]
    fn character_list_chunk_with_one_character() {
        // A character with empty equipment, built field by field like the server's `Character.Write`.
        let mut w = Writer::new();
        w.bool(true).u8(1);
        w.u32(5).string("Ab").u8(1).u8(1).u8(1).i32(10).i32(10).u32(8).u32(0).string("").u32(0).u32(0);
        for _ in 0..slot::COUNT {
            w.u32(0);
        }
        w.u8(1).u8(11).u8(11).long_position(Pos::new(1.0, 2.0, 3.0));
        crate::model::ModelParams::default_face().write(&mut w);
        w.i16(0).i64(0).i16(0).i64(0).i32(0).i64(0).i32(0).i64(0).i64(0).i64(0).i16(0).i32(0).i16(0);
        w.i64(0).i64(0).i64(0).i32(0).i64(0).i64(0).i64(0).bool(false).i32(0).i32(0).i32(0).i64(0).u8(0);
        let m = ServerMsg::decode(&game_frame(sc::SCCharacterListPacket, &w.finish())).unwrap();
        let ServerMsg::CharacterList { last, characters } = m else { panic!("not a list") };
        assert!(last);
        assert_eq!(characters.len(), 1);
        assert_eq!(characters[0].name, "Ab");
        assert_eq!(characters[0].zone_id, 8);
    }
    #[test]
    fn cooldown_dd04_preserves_empty_nonempty_reset_and_tail_order() {
        use crate::skill::{Cooldown, SkillCooldowns};
        let expected = SkillCooldowns {
            skills: vec![Cooldown {
                id: 18132,
                total_ms: 60_000,
                remaining_ms: 4321,
            }],
            tags: vec![Cooldown {
                id: 77,
                total_ms: 1000,
                remaining_ms: 999,
            }],
        };
        let mut body = Writer::new();
        body.i32(1)
            .u32(18132)
            .u32(60_000)
            .u32(4321)
            .i32(1)
            .u32(77)
            .u32(1000)
            .u32(999);
        let body = body.finish();
        assert_eq!(
            decode(sc::SCCooldownsPacket, &body),
            ServerMsg::Cooldowns(expected.clone())
        );
        assert_eq!(ServerMsg::Cooldowns(expected.clone()).name(), "SCCooldowns");
        for len in 0..body.len() {
            assert!(ServerMsg::decode(&game_frame(sc::SCCooldownsPacket, &body[..len])).is_err());
        }
        let mut batch = Writer::new();
        batch.u16(0).u16(sc::SCCooldownsPacket).raw(&[0; 8]);
        batch.u16(0).u16(sc::SCCooldownsPacket).raw(&body);
        batch
            .u16(0)
            .u16(sc::SCSkillCooldownResetPacket)
            .bc(5)
            .u32(18132)
            .u32(77)
            .bool(true);
        batch.u16(0).u16(sc::SCCooldownsPacket).raw(&[0; 8]);
        batch.u16(0).u16(sc::SCSetBreathPacket).u32(u32::MAX);
        let mut frame = vec![MAGIC, LEVEL_COMPRESSED, 5, 0];
        frame.extend(stored(&batch.finish(), true));
        assert_eq!(
            ServerMsg::decode(&frame).unwrap(),
            ServerMsg::Batch(vec![
                ServerMsg::Cooldowns(SkillCooldowns::default()),
                ServerMsg::Cooldowns(expected),
                ServerMsg::SkillCooldownReset {
                    obj_id: 5,
                    skill_id: 18132,
                    tag_id: 77,
                    gcd: true
                },
                ServerMsg::Cooldowns(SkillCooldowns::default()),
                ServerMsg::SetBreath { breath: u32::MAX },
            ])
        );
        // A corrupt count is skipped: the batch resumes where the rest decodes to its end.
        let mut batch = Writer::new();
        batch.u16(0).u16(sc::SCSetBreathPacket).u32(7);
        batch.u16(0).u16(sc::SCCooldownsPacket).i32(-1).i32(0);
        batch.u16(0).u16(sc::SCSetBreathPacket).u32(8);
        assert!(matches!(ServerMsg::decode_batch(&batch.finish(),3).as_slice(),
            [ServerMsg::SetBreath{breath:7},ServerMsg::DecodeFailed{..},ServerMsg::SetBreath{breath:8}]));
    }

    #[test]
    fn remove_buff_native_lane_wire_golden() {
        for (obj_id,buff_index,reason,expected) in [
            (0x010203,0x89abcdef,0,vec![3,2,1,239,205,171,137,0]),
            (0xffffff,0,9,vec![255,255,255,0,0,0,0,9]),
            (1,u32::MAX,255,vec![1,0,0,255,255,255,255,255])] {
            let packet=ClientMsg::RemoveBuff{obj_id,buff_index,reason};
            assert_eq!(packet.opcode(),0x55);
            assert_eq!(packet.encode_body(),expected);
        }
    }

    #[test]
    fn native_buff_updated_signed_fields_truncation_and_dd04_alignment() {
        // Original397c63f0 executed read/write, archive primitives mocked; independent of Writer.
        let native_golden=[3,2,1,255,255,255,255,0,0,0,128,255,255,255,127];
        assert_eq!(decode(sc::SCBuffUpdatedPacket,&native_golden),ServerMsg::BuffUpdated{obj_id:0x010203,index:u32::MAX,stacks:i32::MIN,charged:i32::MAX});
        for (stacks,charged) in [(i32::MIN,i32::MAX),(0,-1),(99,-99)] {
            let mut w=Writer::new();w.bc(0x010203).u32(u32::MAX).i32(stacks).i32(charged);let body=w.finish();
            assert_eq!(body.len(),15);let expected=ServerMsg::BuffUpdated{obj_id:0x010203,index:u32::MAX,stacks,charged};
            assert_eq!(decode(sc::SCBuffUpdatedPacket,&body),expected);assert_eq!(expected.name(),"SCBuffUpdated");
            for n in 0..body.len(){assert!(ServerMsg::decode(&game_frame(sc::SCBuffUpdatedPacket,&body[..n])).is_err());}
            let mut batch=Writer::new();batch.u16(0).u16(sc::SCBuffUpdatedPacket).raw(&body);
            batch.u16(0).u16(sc::SCBuffUpdatedPacket).raw(&body);
            batch.u16(0).u16(sc::SCBuffRemovedPacket).bc(0x010203).u32(7);
            assert_eq!(ServerMsg::decode_batch(&batch.finish(),3),vec![expected.clone(),expected,ServerMsg::BuffRemoved{obj_id:0x010203,index:7}]);
        }
    }

    #[test]
    fn socket_result_native_goldens_raw_bytes_truncation_and_dd04_alignment() {
        // Actual397b39b0/397b3a40 field walk with canonical archive bool primitive, independent Writer.
        let gem=[1,8,7,6,5,4,3,2,1,0xdd,0xcc,0xbb,0xaa,1];
        let stone=&gem[..13];
        let expected_gem=ServerMsg::LunagemSocketingResult{raw_result:1,item_id:0x0102030405060708,template_id:0xaabbccdd,raw_install:1};
        let expected_stone=ServerMsg::LunastoneSocketingResult{raw_result:1,item_id:0x0102030405060708,template_id:0xaabbccdd};
        for (opcode,body,expected) in [(sc::SCItemSocketingLunagemResultPacket,&gem[..],expected_gem.clone()),(sc::SCItemSocketingLunastoneResultPacket,stone,expected_stone.clone())] {
            assert_eq!(decode(opcode,body),expected);
            for n in 0..body.len(){assert!(ServerMsg::decode(&game_frame(opcode,&body[..n])).is_err());}
        }
        let mut noncanonical=gem;noncanonical[0]=255;noncanonical[13]=254;
        assert_eq!(decode(sc::SCItemSocketingLunagemResultPacket,&noncanonical),ServerMsg::LunagemSocketingResult{raw_result:255,item_id:0x0102030405060708,template_id:0xaabbccdd,raw_install:254});
        assert_eq!(decode(sc::SCItemSocketingLunastoneResultPacket,&noncanonical[..13]),ServerMsg::LunastoneSocketingResult{raw_result:255,item_id:0x0102030405060708,template_id:0xaabbccdd});
        let mut batch=Writer::new();batch.u16(0).u16(sc::SCItemSocketingLunagemResultPacket).raw(&gem);
        batch.u16(0).u16(sc::SCItemSocketingLunastoneResultPacket).raw(stone);
        batch.u16(0).u16(sc::SCItemSocketingLunagemResultPacket).raw(&gem);
        batch.u16(0).u16(sc::SCBuffRemovedPacket).bc(9).u32(7);
        assert_eq!(ServerMsg::decode_batch(&batch.finish(),4),vec![expected_gem.clone(),expected_stone,expected_gem,ServerMsg::BuffRemoved{obj_id:9,index:7}]);
    }

    #[test]
    fn grade_result_inline_item_and_broadcast_dd04_alignment() {
        // Non-null existing Item wire, independently assembled with no detail payload.
        let mut item=Writer::new();item.u32(123).u64(456).u8(6).u8(128).i32(-2).u8(0).i64(i64::MIN).i32(-1).u32(789).u8(255).i64(-2).i64(i64::MAX);
        let item=item.finish();let mut result=vec![255];result.extend_from_slice(&item);result.extend_from_slice(&[1,6]);
        let expected=match decode(sc::SCItemGradeEnchantResultPacket,&result){ServerMsg::GradeEnchantResult(v)=>v,_=>panic!()};
        assert_eq!(expected.item_wire,item);let decoded=expected.item.as_ref().unwrap();assert_eq!((decoded.template_id,decoded.id,decoded.count),(123,456,-2));assert_eq!(decoded.create_time,i64::MIN);
        let mut broadcast=vec![4,0,255,b'A',0,b'B'];broadcast.extend_from_slice(&result);
        let announced=crate::item_effect::GradeEnchantBroadcast{raw_name:vec![255,b'A',0,b'B'],result:expected.clone()};
        assert_eq!(decode(sc::SCItemGradeEnchantBroadcastPacket,&broadcast),ServerMsg::GradeEnchantBroadcast(announced.clone()));
        for (opcode,body) in [(sc::SCItemGradeEnchantResultPacket,&result),(sc::SCItemGradeEnchantBroadcastPacket,&broadcast)] {for n in 0..body.len(){assert!(ServerMsg::decode(&game_frame(opcode,&body[..n])).is_err());}}
        let mut batch=Writer::new();batch.u16(0).u16(sc::SCItemGradeEnchantResultPacket).raw(&result).u16(0).u16(sc::SCItemGradeEnchantBroadcastPacket).raw(&broadcast).u16(0).u16(sc::SCBuffRemovedPacket).bc(9).u32(7);
        assert_eq!(ServerMsg::decode_batch(&batch.finish(),3),vec![ServerMsg::GradeEnchantResult(expected),ServerMsg::GradeEnchantBroadcast(announced),ServerMsg::BuffRemoved{obj_id:9,index:7}]);
    }

    #[test]
    fn ucc_result_native_golden_truncation_and_dd04_alignment() {
        let raw=[8,7,6,5,4,3,2,1,0xdd,0xcc,0xbb,0xaa,0x18,0x17,0x16,0x15,0x14,0x13,0x12,0x11];
        let expected=ServerMsg::ItemUccDataChanged{ucc_id:0x0102030405060708,player_id:0xaabbccdd,target_item_id:0x1112131415161718};
        assert_eq!(decode(sc::SCItemUccDataChangedPacket,&raw),expected);
        for n in 0..raw.len(){assert!(ServerMsg::decode(&game_frame(sc::SCItemUccDataChangedPacket,&raw[..n])).is_err());}
        assert_eq!(decode(sc::SCItemUccDataChangedPacket,&[255;20]),ServerMsg::ItemUccDataChanged{ucc_id:u64::MAX,player_id:u32::MAX,target_item_id:u64::MAX});
        assert_eq!(decode(sc::SCItemUccDataChangedPacket,&[0;20]),ServerMsg::ItemUccDataChanged{ucc_id:0,player_id:0,target_item_id:0});
        let mut batch=Writer::new();batch.u16(0).u16(sc::SCItemUccDataChangedPacket).raw(&raw).u16(0).u16(sc::SCItemUccDataChangedPacket).raw(&raw).u16(0).u16(sc::SCBuffRemovedPacket).bc(9).u32(7);
        assert_eq!(ServerMsg::decode_batch(&batch.finish(),3),vec![expected.clone(),expected,ServerMsg::BuffRemoved{obj_id:9,index:7}]);
    }

    #[test]
    fn schedule_lane32_and_raw_sent_dd04_alignment() {
        let row=[0x44,0x33,0x22,0x11,255,0xfe,255,255,255,0,0,0,0,0,0,0,128];let mut body=vec![32];for _ in 0..32{body.extend_from_slice(&row);}
        let entry=crate::schedule::ScheduleItem{schedule_type:0x11223344,raw_gave:255,accumulated:-2,raw_updated:i64::MIN};
        let expected=ServerMsg::ScheduleItems{declared_count:32,items:vec![entry;32]};assert_eq!(decode(sc::SCScheduleItemUpdatePacket,&body),expected);
        let sent=[255,255,255,255,255];assert_eq!(decode(sc::SCScheduleItemSentPacket,&sent),ServerMsg::ScheduleItemSent{schedule_type:u32::MAX,raw_by_mail:255});
        for n in 0..sent.len(){assert!(ServerMsg::decode(&game_frame(sc::SCScheduleItemSentPacket,&sent[..n])).is_err());}
        let mut batch=Writer::new();batch.u16(0).u16(sc::SCScheduleItemUpdatePacket).raw(&body).u16(0).u16(sc::SCScheduleItemSentPacket).raw(&sent).u16(0).u16(sc::SCBuffRemovedPacket).bc(9).u32(7);
        assert_eq!(ServerMsg::decode_batch(&batch.finish(),3),vec![expected,ServerMsg::ScheduleItemSent{schedule_type:u32::MAX,raw_by_mail:255},ServerMsg::BuffRemoved{obj_id:9,index:7}]);
    }

    #[test]
    fn detail_updates_preserve_repeated_dd04_order_and_following_packet() {
        let mut inner=Writer::new();
        for kind in [1u8,2,255] {
            inner.u16(0).u16(0x93).u64(123).u8(2).u8(3).u8(kind);
            inner.raw(&vec![0;if kind==1 {55}else if kind==2 {29}else {0}]);
        }
        inner.u16(0).u16(0xa3).u16(7);
        let mut frame=Writer::new();frame.u8(0xdd).u8(LEVEL_COMPRESSED).u16(3).raw(&stored(&inner.finish(),true));
        let ServerMsg::Batch(messages)=ServerMsg::decode(&frame.finish()).unwrap() else {panic!()};assert_eq!(messages.len(),4);
        for (m,kind) in messages[..3].iter().zip([1,2,255]) {assert!(matches!(m,ServerMsg::ItemDetailUpdated(u) if u.raw_detail[0]==kind &&u.item_id==123));}
        assert_eq!(messages[3],ServerMsg::SkillEnded{tl:7});
        let body=[0x7b,0,0,0,0,0,0,0,2,3,3,0xff,0xff,0xff,0xff,0xf2,0xfe];
        for n in 0..body.len() {
            let mut frame=Writer::new();frame.raw(&[0xdd,1,0,0]).u16(0x93).raw(&body[..n]);assert!(ServerMsg::decode(&frame.finish()).is_err());
        }
    }

    #[test]
    fn native_equipment_ids_exact_codec_truncation_and_following_packet() {
        // Actual397b38b0 oracle: BC3 then exactly28 independent u64s.
        let ids=std::array::from_fn(|i|match i%5 {0=>0,1=>u64::MAX,_=>0x1122334400000000+i as u64});
        let mut raw=vec![0x56,0x34,0x12];
        for id in ids {raw.extend_from_slice(&id.to_le_bytes());}
        let expected=ServerMsg::EquipmentIdsChanged{obj_id:0x123456,ids};
        assert_eq!(raw.len(),227);assert_eq!(decode(0x95,&raw),expected);
        for n in 0..227 {assert!(ServerMsg::decode(&game_frame(0x95,&raw[..n])).is_err());}
        let mut batch=Writer::new();
        for _ in 0..2 {batch.u16(0).u16(0x95).raw(&raw);}
        batch.u16(0).u16(0xa3).u16(7);
        assert_eq!(ServerMsg::decode_batch(&batch.finish(),3),vec![expected.clone(),expected,ServerMsg::SkillEnded{tl:7}]);
    }

    #[test]
    fn specialty_ratio_native_lane_signed_goldens_and_compressed_alignment() {
        for (body,value) in [([0,0,0,0],0),([117,0,0,0],117),([255;4],-1),([0,0,0,128],i32::MIN),([255,255,255,127],i32::MAX)] {
            assert_eq!(decode(0x99,&body),ServerMsg::SpecialtyRatio(value));
            for n in 0..4 {assert!(ServerMsg::decode(&game_frame(0x99,&body[..n])).is_err());}
        }
        // Lane craft_store_tests literal117 matches native397aec70 scalar walk.
        let mut inner=Writer::new();inner.u16(0).u16(0x99).raw(&[117,0,0,0]);
        inner.u16(0).u16(0x99).raw(&[255;4]);inner.u16(0).u16(0xa3).u16(7);
        let mut frame=Writer::new();frame.u8(0xdd).u8(LEVEL_COMPRESSED).u16(3).raw(&stored(&inner.finish(),true));
        assert_eq!(ServerMsg::decode(&frame.finish()).unwrap(),ServerMsg::Batch(vec![ServerMsg::SpecialtyRatio(117),ServerMsg::SpecialtyRatio(-1),ServerMsg::SkillEnded{tl:7}]));
    }

}
