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
        CSSendChatMessagePacket = 0x063,
        CSMoveUnitPacket = 0x089,
        CSTeleportEndedPacket = 0x0b4,
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
        SCCreateCharacterResponsePacket = 0x038,
        SCDeleteCharacterResponsePacket = 0x039,
        SCCharacterDeletedPacket = 0x03b,
        SCCancelCharacterDeleteResponsePacket = 0x03c,
        SCCharacterCreationFailedPacket = 0x03d,
        SCCharacterListPacket = 0x03e,
        SCRaceCongestionPacket = 0x03f,
        SCCharacterStatePacket = 0x040,
        SCUnitStatePacket = 0x069,
        SCUnitsRemovedPacket = 0x06a,
        SCOneUnitMovementPacket = 0x06c,
        SCUnitDeathPacket = 0x071,
        SCTeleportUnitPacket = 0x072,
        SCUnitPointsPacket = 0x0ba,
        SCChatMessagePacket = 0x0ce,
        SCTimeOfDayPacket = 0x0f0,
        SCDetailedTimeOfDayPacket = 0x0f1,
        SCErrorMsgPacket = 0x10a,
        SCDoodadCreatedPacket = 0x10c,
        SCDoodadRemovedPacket = 0x10d,
        SCDoodadPhaseChangedPacket = 0x10f,
        SCDoodadsCreatedPacket = 0x112,
        SCDoodadsRemovedPacket = 0x113,
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
