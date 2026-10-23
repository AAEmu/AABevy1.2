//! Account and lobby messages: the configuration the server sends at login, faction relations,
//! the character list refresh, the labor power managed character, connection restrictions and
//! account warnings.
//!
//! The layouts are the lane server's writers, checked against the 1.2 client's readers:
//!
//! | packet | body |
//! |---|---|
//! | `SCTrionConfig` 0x007 | `bool active, str auth url, str platform url, str commerce url` |
//! | `SCFactionRelationList` 0x009 | `bool ui request, u8 count`, `u32 faction, u32 other, u8 state, i64 expires, i64 kind, u8 next state` each |
//! | `SCLpManaged` 0x048 | `u32 character` |
//! | `SCAccountWarned` 0x1a1 | `u8 source, str message` |
//! | `SCRefreshInCharacterList` 0x1cd | `u8 x9` race congestion |
//! | `SCResultRestrictCheck` 0x1ce | `u32 character, u8 code, u8 result` |
//! | `SCAccountAttributeConfig` 0x215 | `bool x2` |
//! | `SCLevelRestrictionConfig` 0x217 | `u8 auction search, u8 auction bid, u8 auction post, u8 trade, u8 mail, u8 x15 limits` |
//!
//! The 1.2 client answers `SCRefreshInCharacterList` (the reply of `CSRefreshInCharacterList`,
//! sent by `X2LoginCharacter:RequestCharacterListRefresh`, 39484860) by storing the congestion and
//! raising `LOGIN_CHARACTER_UPDATED("refreshed")` (391d2ff0), and `SCLpManaged` (the reply of
//! `CSSetLpManageCharacter`) by raising `LP_MANAGE_CHARACTER_CHANGED(character, index)` (394d33e0).
use crate::wire::{Reader, Result, WireError, Writer};

/// Lobby requests of the 1.2 client (`Command::Account`, sent on the character select screen).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccountCommand {
    /// `CSRefreshInCharacterList` 0x021 (`X2LoginCharacter:RequestCharacterListRefresh`); the
    /// answer is [`AccountEvent::CharacterListRefreshed`].
    RefreshInCharacterList,
    /// `CSSetLpManageCharacter` 0x0fe (`X2LoginCharacter:RequestLpManageCharacter`); the answer
    /// is [`AccountEvent::LpManaged`].
    SetLpManageCharacter { character_id: u32 },
}

impl AccountCommand {
    pub fn opcode(&self) -> u16 {
        match self {
            Self::RefreshInCharacterList => 0x021,
            Self::SetLpManageCharacter { .. } => 0x0fe,
        }
    }

    pub fn write(&self, w: &mut Writer) {
        if let Self::SetLpManageCharacter { character_id } = self {
            w.u32(*character_id);
        }
    }
}

pub mod op {
    pub const TRION_CONFIG: u16 = 0x007;
    pub const FACTION_RELATION_LIST: u16 = 0x009;
    pub const LP_MANAGED: u16 = 0x048;
    pub const ACCOUNT_WARNED: u16 = 0x1a1;
    pub const REFRESH_IN_CHARACTER_LIST: u16 = 0x1cd;
    pub const RESULT_RESTRICT_CHECK: u16 = 0x1ce;
    pub const ACCOUNT_ATTRIBUTE_CONFIG: u16 = 0x215;
    pub const LEVEL_RESTRICTION_CONFIG: u16 = 0x217;
}

/// A relation between two factions (`FactionRelation`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FactionRelation {
    pub faction_id: u32,
    pub other_id: u32,
    pub state: u8,
    /// Unix time the relation expires (0 = never).
    pub expires: i64,
    pub kind: i64,
    pub next_state: u8,
}

/// The level limits of trading features (`SCLevelRestrictionConfig`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LevelRestrictions {
    pub auction_search: u8,
    pub auction_bid: u8,
    pub auction_post: u8,
    pub trade: u8,
    pub mail: u8,
    pub limits: [u8; 15],
}

#[derive(Clone, Debug, PartialEq)]
pub enum AccountEvent {
    TrionConfig { active: bool, auth_url: String, platform_url: String, commerce_url: String },
    FactionRelations { ui_request: bool, relations: Vec<FactionRelation> },
    /// The character whose labor power the account manages (`CSSetLpManageCharacter`).
    LpManaged { character_id: u32 },
    /// A warning for the account (`source` 0..4 picks the client's message box).
    Warned { source: u8, message: String },
    /// The answer of `Command::RefreshInCharacterList`: the race congestion of the world.
    CharacterListRefreshed { race_congestion: [u8; 9] },
    /// The answer of a connection restriction check (`code` 1: entering with a character, 0: the account).
    RestrictCheck { character_id: u32, code: u8, result: u8 },
    AccountAttributes([bool; 2]),
    LevelRestrictions(LevelRestrictions),
}

impl AccountEvent {
    pub fn handles(opcode: u16) -> bool {
        use op::*;
        matches!(
            opcode,
            TRION_CONFIG
                | FACTION_RELATION_LIST
                | LP_MANAGED
                | ACCOUNT_WARNED
                | REFRESH_IN_CHARACTER_LIST
                | RESULT_RESTRICT_CHECK
                | ACCOUNT_ATTRIBUTE_CONFIG
                | LEVEL_RESTRICTION_CONFIG
        )
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::TrionConfig { .. } => "SCTrionConfig",
            Self::FactionRelations { .. } => "SCFactionRelationList",
            Self::LpManaged { .. } => "SCLpManaged",
            Self::Warned { .. } => "SCAccountWarned",
            Self::CharacterListRefreshed { .. } => "SCRefreshInCharacterList",
            Self::RestrictCheck { .. } => "SCResultRestrictCheck",
            Self::AccountAttributes(_) => "SCAccountAttributeConfig",
            Self::LevelRestrictions(_) => "SCLevelRestrictionConfig",
        }
    }

    pub fn read(opcode: u16, r: &mut Reader) -> Result<Self> {
        Ok(match opcode {
            op::TRION_CONFIG => {
                Self::TrionConfig { active: r.bool()?, auth_url: r.string()?, platform_url: r.string()?, commerce_url: r.string()? }
            }
            op::FACTION_RELATION_LIST => {
                let ui_request = r.bool()?;
                let count = r.u8()?;
                let relations = (0..count)
                    .map(|_| {
                        Ok(FactionRelation {
                            faction_id: r.u32()?,
                            other_id: r.u32()?,
                            state: r.u8()?,
                            expires: r.i64()?,
                            kind: r.i64()?,
                            next_state: r.u8()?,
                        })
                    })
                    .collect::<Result<_>>()?;
                Self::FactionRelations { ui_request, relations }
            }
            op::LP_MANAGED => Self::LpManaged { character_id: r.u32()? },
            op::ACCOUNT_WARNED => Self::Warned { source: r.u8()?, message: r.string()? },
            op::REFRESH_IN_CHARACTER_LIST => Self::CharacterListRefreshed { race_congestion: r.bytes(9)?.try_into().expect("nine bytes") },
            op::RESULT_RESTRICT_CHECK => Self::RestrictCheck { character_id: r.u32()?, code: r.u8()?, result: r.u8()? },
            op::ACCOUNT_ATTRIBUTE_CONFIG => Self::AccountAttributes([r.bool()?, r.bool()?]),
            op::LEVEL_RESTRICTION_CONFIG => Self::LevelRestrictions(LevelRestrictions {
                auction_search: r.u8()?,
                auction_bid: r.u8()?,
                auction_post: r.u8()?,
                trade: r.u8()?,
                mail: r.u8()?,
                limits: r.bytes(15)?.try_into().expect("fifteen bytes"),
            }),
            opcode => return Err(WireError::Invalid(format!("not an account packet: {opcode:#05x}"))),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lobby_requests_encode() {
        assert_eq!(AccountCommand::RefreshInCharacterList.opcode(), 0x021);
        let mut w = Writer::new();
        AccountCommand::RefreshInCharacterList.write(&mut w);
        assert!(w.as_bytes().is_empty());
        let lp = AccountCommand::SetLpManageCharacter { character_id: 0x01020304 };
        assert_eq!(lp.opcode(), 0x0fe);
        lp.write(&mut w);
        assert_eq!(w.as_bytes(), [4, 3, 2, 1]);
    }

    fn read(opcode: u16, w: &Writer) -> AccountEvent {
        let body = w.as_bytes();
        for n in 0..body.len() {
            assert!(AccountEvent::read(opcode, &mut Reader::new(&body[..n])).is_err(), "{opcode:#x} cut {n}");
        }
        AccountEvent::read(opcode, &mut Reader::new(body)).unwrap()
    }

    #[test]
    fn login_configuration_decodes() {
        let mut w = Writer::new();
        w.bool(false).string("").string("p").string("");
        assert_eq!(
            read(op::TRION_CONFIG, &w),
            AccountEvent::TrionConfig { active: false, auth_url: String::new(), platform_url: "p".into(), commerce_url: String::new() }
        );
        let mut w = Writer::new();
        w.bool(true).bool(false);
        assert_eq!(read(op::ACCOUNT_ATTRIBUTE_CONFIG, &w), AccountEvent::AccountAttributes([true, false]));
        let mut w = Writer::new();
        w.raw(&[10, 11, 12, 13, 14]).raw(&[1; 15]);
        let levels = LevelRestrictions { auction_search: 10, auction_bid: 11, auction_post: 12, trade: 13, mail: 14, limits: [1; 15] };
        assert_eq!(read(op::LEVEL_RESTRICTION_CONFIG, &w), AccountEvent::LevelRestrictions(levels));
        let mut w = Writer::new();
        w.bool(false).u8(1).u32(101).u32(102).u8(2).i64(0).i64(0).u8(0);
        let relation = FactionRelation { faction_id: 101, other_id: 102, state: 2, ..FactionRelation::default() };
        assert_eq!(read(op::FACTION_RELATION_LIST, &w), AccountEvent::FactionRelations { ui_request: false, relations: vec![relation] });
    }

    #[test]
    fn lobby_replies_decode() {
        let mut w = Writer::new();
        w.raw(&[0, 1, 2, 0, 0, 0, 0, 0, 0]);
        assert_eq!(
            read(op::REFRESH_IN_CHARACTER_LIST, &w),
            AccountEvent::CharacterListRefreshed { race_congestion: [0, 1, 2, 0, 0, 0, 0, 0, 0] }
        );
        let mut w = Writer::new();
        w.u32(7);
        assert_eq!(read(op::LP_MANAGED, &w), AccountEvent::LpManaged { character_id: 7 });
        let mut w = Writer::new();
        w.u32(7).u8(1).u8(0);
        assert_eq!(read(op::RESULT_RESTRICT_CHECK, &w), AccountEvent::RestrictCheck { character_id: 7, code: 1, result: 0 });
        let mut w = Writer::new();
        w.u8(2).string("Premium ...");
        assert_eq!(read(op::ACCOUNT_WARNED, &w), AccountEvent::Warned { source: 2, message: "Premium ...".into() });
    }
}
