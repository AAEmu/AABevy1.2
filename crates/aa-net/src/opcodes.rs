//! Opcodes of the 1.2 (r208022) protocol that this client sends or understands.
//!
//! Copied from the server's `opcodes` tables (`CSOffsets`, `SCOffsets`, `PPOffsets`, `LCOffsets`,
//! `CLOffsets`); the names match the server so they can be grepped across both code bases.

macro_rules! opcodes {
    ($(#[$m:meta])* $vis:vis mod $name:ident { $($op:ident = $val:literal,)* }) => {
        $(#[$m])*
        #[allow(non_upper_case_globals, dead_code)]
        $vis mod $name {
            $(pub const $op: u16 = $val;)*

            /// The packet name of `opcode`, for logs.
            pub fn name(opcode: u16) -> &'static str {
                match opcode {
                    $($val => stringify!($op),)*
                    _ => "?",
                }
            }
        }
    };
}

opcodes! {
    /// Client to login server (`CA*`).
    pub mod ca {
        CARequestAuthTrionPacket = 0x004,
        CAListWorldPacket = 0x00b,
        CAEnterWorldPacket = 0x00c,
        CACancelEnterWorldPacket = 0x00d,
        CARequestReconnectPacket = 0x00e,
    }
}

opcodes! {
    /// Login server to client (`AC*`).
    pub mod ac {
        ACJoinResponsePacket = 0x000,
        ACChallengePacket = 0x002,
        ACAuthResponsePacket = 0x003,
        ACChallenge2Packet = 0x004,
        ACEnterOtpPacket = 0x005,
        ACShowArsPacket = 0x006,
        ACEnterPcCertPacket = 0x007,
        ACWorldListPacket = 0x008,
        ACWorldQueuePacket = 0x009,
        ACWorldCookiePacket = 0x00a,
        ACEnterWorldDeniedPacket = 0x00b,
        ACLoginDeniedPacket = 0x00c,
        ACAccountWarnedPacket = 0x00d,
    }
}

opcodes! {
    /// Client to game server, level 1 (`CS*`).
    pub mod cs {
        X2EnterWorldPacket = 0x000,
        CSLeaveWorldPacket = 0x001,
        CSCancelLeaveWorldPacket = 0x002,
        CSListCharacterPacket = 0x020,
        CSCreateCharacterPacket = 0x022,
        CSDeleteCharacterPacket = 0x024,
        CSSelectCharacterPacket = 0x025,
        CSSpawnCharacterPacket = 0x026,
        CSCancelCharacterDeletePacket = 0x027,
        CSNotifyInGamePacket = 0x029,
        CSNotifyInGameCompletedPacket = 0x02a,
        CSChangeTargetPacket = 0x02c,
        CSResurrectCharacterPacket = 0x04e,
        CSStartSkillPacket = 0x052,
        CSStopCastingPacket = 0x054,
        CSSendChatMessagePacket = 0x063,
        CSMoveUnitPacket = 0x089,
        CSLootOpenBagPacket = 0x08e,
        CSLootItemPacket = 0x08f,
        CSLootCloseBagPacket = 0x090,
        CSTeleportEndedPacket = 0x0b4,
        CSUpdateActionSlotPacket = 0x0b6,
    }
}

opcodes! {
    /// Game server to client, level 1 (`SC*`).
    pub mod sc {
        X2EnterWorldResponsePacket = 0x000,
        SCReconnectAuthPacket = 0x001,
        SCPrepareLeaveWorldPacket = 0x002,
        SCLeaveWorldGrantedPacket = 0x003,
        SCLeaveWorldCanceledPacket = 0x004,
        SCInitialConfigPacket = 0x006,
        SCFactionListPacket = 0x008,
        SCFactionRelationListPacket = 0x009,
        SCUnitFactionChangedPacket = 0x01b,
        SCUnitExpeditionChangedPacket = 0x01c,
        SCCreateCharacterResponsePacket = 0x038,
        SCDeleteCharacterResponsePacket = 0x039,
        SCCharacterDeletedPacket = 0x03b,
        SCCancelCharacterDeleteResponsePacket = 0x03c,
        SCCharacterCreationFailedPacket = 0x03d,
        SCCharacterListPacket = 0x03e,
        SCRaceCongestionPacket = 0x03f,
        SCCharacterStatePacket = 0x040,
        SCNotifyResurrectionPacket = 0x041,
        SCCharacterResurrectedPacket = 0x042,
        SCCharacterLaborPowerChangedPacket = 0x045,
        SCBmPointPacket = 0x046,
        SCCooldownsPacket = 0x04a,
        SCCharacterInvenInitPacket = 0x04b,
        SCCharacterInvenContentsPacket = 0x04c,
        SCInvenExpandedPacket = 0x04d,
        SCUnitStatePacket = 0x069,
        SCUnitsRemovedPacket = 0x06a,
        SCOneUnitMovementPacket = 0x06c,
        SCUnitDeathPacket = 0x071,
        SCTeleportUnitPacket = 0x072,
        SCTargetChangedPacket = 0x084,
        SCCombatEngagedPacket = 0x085,
        SCCombatClearedPacket = 0x086,
        SCItemTaskSuccessPacket = 0x090,
        SCUnitEquipmentsChangedPacket = 0x094,
        SCSyncItemLifespanPacket = 0x098,
        SCSkillStartedPacket = 0x0a1,
        SCSkillFiredPacket = 0x0a2,
        SCSkillEndedPacket = 0x0a3,
        SCSkillStoppedPacket = 0x0a4,
        SCCastingStoppedPacket = 0x0a5,
        SCUnitDamagedPacket = 0x0a7,
        SCUnitHealedPacket = 0x0a8,
        SCCombatTextPacket = 0x0ab,
        SCSkillCooldownResetPacket = 0x0ad,
        SCPlotEventPacket = 0x0ae,
        SCPlotEndedPacket = 0x0af,
        SCPlotCastingStoppedPacket = 0x0b1,
        SCPlotChannelingStoppedPacket = 0x0b2,
        SCOnOffSnowPacket = 0x0b4,
        SCBuffCreatedPacket = 0x0b6,
        SCBuffRemovedPacket = 0x0b7,
        SCBuffUpdatedPacket = 0x0b9,
        SCUnitPointsPacket = 0x0ba,
        SCChatMessagePacket = 0x0ce,
        SCJoinedChatChannelPacket = 0x0cc,
        SCLeavedChatChannelPacket = 0x0cd,
        SCTimeOfDayPacket = 0x0f0,
        SCDetailedTimeOfDayPacket = 0x0f1,
        SCQuestsPacket = 0x0f2,
        SCCompletedQuestsPacket = 0x0f3,
        SCLootableStatePacket = 0x0f6,
        SCUnitLootingStatePacket = 0x0f7,
        SCLootBagDataPacket = 0x0f8,
        SCLootItemTookPacket = 0x0f9,
        SCLootItemFailedPacket = 0x0fa,
        SCExpChangedPacket = 0x0fe,
        SCAbilityExpChangedPacket = 0x0ff,
        SCRecoverableExpPacket = 0x100,
        SCLevelChangedPacket = 0x102,
        SCSkillLearnedPacket = 0x105,
        SCSkillUpgradedPacket = 0x106,
        SCBuffLearnedPacket = 0x107,
        SCSkillsResetPacket = 0x108,
        SCAbilitySwappedPacket = 0x109,
        SCErrorMsgPacket = 0x10a,
        SCDoodadCreatedPacket = 0x10c,
        SCDoodadRemovedPacket = 0x10d,
        SCDoodadPhaseChangedPacket = 0x10f,
        SCDoodadsCreatedPacket = 0x112,
        SCDoodadsRemovedPacket = 0x113,
        SCActionSlotsPacket = 0x12d,
        SCQuestContextStartedPacket = 0x147,
        SCQuestContextUpdatedPacket = 0x149,
        SCQuestContextCompletedPacket = 0x14b,
        SCQuestContextResetPacket = 0x14c,
        SCCharacterGamePointsPacket = 0x188,
        SCGamePointChangedPacket = 0x189,
        SCKickedPacket = 0x1a0,
        SCAccountInfoPacket = 0x1c2,
        SCGetSlotCountPacket = 0x1e2,
        SCUpdatePremiumPointPacket = 0x201,
    }
}

opcodes! {
    /// Proxy (CryNetwork) channel, level 2, both directions.
    pub mod pp {
        ChangeStatePacket = 0x000,
        FinishStatePacket = 0x001,
        SetGameTypePacket = 0x00f,
        PingPacket = 0x012,
        PongPacket = 0x013,
        FastPingPacket = 0x015,
        FastPongPacket = 0x016,
    }
}

opcodes! {
    /// Client to stream server (`CT*`).
    pub mod ct {
        CTJoinPacket = 0x001,
    }
}

opcodes! {
    /// Stream server to client (`TC*`).
    pub mod tc {
        TCJoinResponsePacket = 0x001,
        TCDoodadStreamPacket = 0x002,
        TCDoodadIdsPacket = 0x003,
        TCDownloadEmblemPacket = 0x004,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_resolve() {
        assert_eq!(sc::name(sc::SCUnitStatePacket), "SCUnitStatePacket");
        assert_eq!(cs::name(0x089), "CSMoveUnitPacket");
        assert_eq!(ac::name(0x3ff), "?");
    }
}
