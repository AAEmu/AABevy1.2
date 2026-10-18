//! Bounded native SCInitialConfig payload, without guessed feature defaults or UI policy.

use crate::session::{Event, Phase};
use crate::wire::{Reader, Result, WireError};

pub const MAX_HOST_BYTES: usize = 259;
pub const MAX_FEATURE_BYTES: usize = 11;
pub const MAX_CANDIDATE_CODES: i32 = 100;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InitialConfig {
    /// Original string bytes, including any embedded NUL or non-UTF-8 bytes.
    pub host: Vec<u8>,
    /// Preserve missing and unknown feature bytes; no constructor defaults are inserted.
    pub feature_bytes: Vec<u8>,
    pub raw_candidate_count: i32,
    pub candidate_codes: Vec<u32>,
    pub initial_lp: i32,
    /// Native order: place house, pay tax, auction, trade, mail, bank, copper.
    /// Retained as bytes rather than converted into inferred app permissions.
    pub service_flags: [u8; 7],
    pub second_price_type: u8,
    pub second_password_max_fail_count: u8,
    pub idle_kick_time: u16,
}

impl InitialConfig {
    pub fn read(r: &mut Reader) -> Result<Self> {
        let host = r.blob()?;
        if host.len() > MAX_HOST_BYTES {
            return Err(WireError::Invalid("initial config host exceeds native buffer".into()));
        }
        let host = host.to_vec();
        let feature_bytes = r.blob()?;
        if feature_bytes.len() > MAX_FEATURE_BYTES {
            return Err(WireError::Invalid("initial config features exceed native buffer".into()));
        }
        let feature_bytes = feature_bytes.to_vec();
        let raw_candidate_count = r.i32()?;
        let candidate_codes = (0..raw_candidate_count.clamp(0, MAX_CANDIDATE_CODES))
            .map(|_| r.u32())
            .collect::<Result<Vec<_>>>()?;
        let initial_lp = r.i32()?;
        let mut service_flags = [0; 7];
        service_flags.copy_from_slice(r.bytes(7)?);
        Ok(Self {
            host,
            feature_bytes,
            raw_candidate_count,
            candidate_codes,
            initial_lp,
            service_flags,
            second_price_type: r.u8()?,
            second_password_max_fail_count: r.u8()?,
            idle_kick_time: r.u16()?,
        })
    }

    /// Lane FeatureSet byte 1. Missing bytes remain unknown, including on short snapshots.
    pub fn player_level_limit(&self) -> Option<u8> {
        self.feature_bytes.get(1).copied()
    }

    /// Lane FeatureSet byte 8; no native runtime producer mapping or XP cap is inferred.
    pub fn mate_level_limit(&self) -> Option<u8> {
        self.feature_bytes.get(8).copied()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "bevy", derive(bevy::ecs::resource::Resource))]
pub struct GameConfig {
    pub received: Option<InitialConfig>,
}

impl GameConfig {
    pub fn apply(&mut self, event: &Event) -> bool {
        match event {
            Event::InitialConfig(config) => {
                let changed = self.received.as_ref() != Some(config);
                self.received = Some(config.clone());
                changed
            }
            Event::LoginSucceeded(_)
            | Event::ReadyToConnect { .. }
            | Event::PhaseChanged(Phase::Connecting | Phase::Authenticating)
            | Event::Disconnected { .. }
            | Event::Kicked { .. }
            | Event::LoginFailed { .. } => self.received.take().is_some(),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::ServerMsg;
    use crate::model::CharacterState;
    use crate::opcodes::sc;
    use crate::wire::Writer;

    fn body(count: i32, features: &[u8]) -> Vec<u8> {
        let mut w = Writer::new();
        w.blob(b"aaemu.local\0\xff").blob(features).i32(count);
        for n in 0..count.clamp(0, MAX_CANDIDATE_CODES) {
            w.u32(if n % 2 == 0 { u32::MAX } else { 0 });
        }
        w.i32(-50).raw(&[0, 1, 2, 255, 1, 0, 0x80]).u8(0xfe).u8(0xff).u16(0xabcd);
        w.finish()
    }

    #[test]
    fn native_config_signed_count_clamp_and_raw_feature_bytes_preserve_following_dd04() {
        for count in [i32::MIN, -1, 0, 1, 99, 100, 101, i32::MAX] {
            let raw = body(count, &[0x81, 55, 0, 0xff, 2, 3, 4, 5, 50, 6, 7]);
            let mut reader = Reader::new(&raw);
            let config = InitialConfig::read(&mut reader).unwrap();
            assert!(reader.is_empty());
            assert_eq!(config.host, b"aaemu.local\0\xff");
            assert_eq!(config.raw_candidate_count, count);
            assert_eq!(config.candidate_codes.len(), count.clamp(0, 100) as usize);
            assert_eq!(config.initial_lp, -50);
            assert_eq!(config.service_flags, [0, 1, 2, 255, 1, 0, 0x80]);
            assert_eq!(
                (config.second_price_type, config.second_password_max_fail_count, config.idle_kick_time),
                (254, 255, 0xabcd)
            );
            assert_eq!((config.player_level_limit(), config.mate_level_limit()), (Some(55), Some(50)));
            let mut inner = Writer::new();
            inner
                .u16(0)
                .u16(sc::SCInitialConfigPacket)
                .raw(&raw)
                .u16(0)
                .u16(sc::SCOnOffSnowPacket)
                .bool(true);
            let inner = inner.finish();
            let len = inner.len() as u16;
            let mut outer = Writer::new();
            outer.raw(&[0xdd, 4]).u16(2).u8(1).u16(len).u16(!len).raw(&inner);
            assert_eq!(
                ServerMsg::decode(&outer.finish()).unwrap(),
                ServerMsg::Batch(vec![ServerMsg::InitialConfig(config), ServerMsg::Snow(true)])
            );
            for cut in 0..raw.len() {
                assert!(InitialConfig::read(&mut Reader::new(&raw[..cut])).is_err(), "count {count}, cut {cut}");
            }
        }
    }

    #[test]
    fn config_bounds_and_short_features_never_invent_defaults() {
        for len in 0..=11 {
            let features = vec![0; len];
            let config = InitialConfig::read(&mut Reader::new(&body(0, &features))).unwrap();
            assert_eq!(config.feature_bytes, features);
            assert_eq!(config.player_level_limit(), (len > 1).then_some(0));
            assert_eq!(config.mate_level_limit(), (len > 8).then_some(0));
        }
        assert!(InitialConfig::read(&mut Reader::new(&body(0, &[0; 12]))).is_err());
        for len in [259, 260] {
            let mut raw = Writer::new();
            raw.blob(&vec![b'h'; len]).raw(&body(0, &[])[15..]);
            assert_eq!(InitialConfig::read(&mut Reader::new(&raw.finish())).is_ok(), len == 259);
        }
        let mut malformed = Writer::new();
        malformed.blob(b"host").blob(&[]).i32(1);
        assert!(InitialConfig::read(&mut Reader::new(&malformed.finish())).is_err());
    }

    #[test]
    fn config_cache_survives_character_lifecycle_but_resets_for_new_connections() {
        let mut state = GameConfig::default();
        assert_eq!(state.received, None);
        let full = InitialConfig::read(&mut Reader::new(&body(0, &[7; 11]))).unwrap();
        state.apply(&Event::InitialConfig(full));
        let config = InitialConfig::read(&mut Reader::new(&body(0, &[9]))).unwrap();
        assert!(state.apply(&Event::InitialConfig(config.clone())));
        assert_eq!(state.received.as_ref().unwrap().mate_level_limit(), None);
        assert!(state.apply(&Event::PhaseChanged(Phase::Connecting)));
        for reset in [Event::PhaseChanged(Phase::Connecting), Event::Disconnected { message: String::new() }] {
            assert!(state.apply(&Event::InitialConfig(config.clone())));
            assert!(!state.apply(&Event::InitialConfig(config.clone())));
            assert!(!state.apply(&Event::CharacterSelected(Box::new(CharacterState::default()))));
            assert!(!state.apply(&Event::LeftWorld { target: 0 }));
            assert_eq!(state.received, Some(config.clone()));
            assert!(state.apply(&reset));
            assert_eq!(state.received, None);
            assert!(!state.apply(&reset));
        }
    }
}
