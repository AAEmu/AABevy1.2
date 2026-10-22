use super::*;

/// Source of a mutable native timer. Every variant is grounded in an actual received packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeCooldownOrigin {
    Snapshot,
    CastingStopped { tl: u16 },
    PlotStopped { tl: u16 },
}

/// Native manager row, separate from original SCCooldowns receipt history.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeCooldown {
    pub cooldown: Cooldown,
    /// Receipt/insertion metadata; never used to project mutable manager time.
    pub updated_at: Instant,
    pub origin: NativeCooldownOrigin,
    pub reset: bool,
}
impl NativeCooldown {
    /// Stored frame remainder. `now` is retained for API compatibility, not projection.
    pub fn remaining_ms_at(&self, _now: Instant) -> u32 {
        if self.reset { 0 } else { self.cooldown.remaining_ms }
    }

}
impl Skills {
    /// Advance once AFTER queued packet folds and BEFORE HUD queries. Uses the native
    /// wrapping timer0 millisecond domain; zero now suppresses delta but still stores zero.
    /// Character/connection resets preserve this process-frame clock, not runtime rows.
    /// Returns whether any stored remainder changed (the frame clock always updates).
    pub fn advance_native_frame(&mut self, now_ms: u32) -> bool {
        let delta = if now_ms == 0 { 0 } else { now_ms.wrapping_sub(self.native_last_frame_ms) };
        self.native_last_frame_ms = now_ms;
        let mut changed = false;
        for row in self.native_skill_cooldowns.iter_mut().chain(self.native_tag_cooldowns.iter_mut()) {
            let remaining = row.cooldown.remaining_ms.saturating_sub(delta);
            changed |= remaining != row.cooldown.remaining_ms;
            row.cooldown.remaining_ms = remaining;
        }
        changed
    }

    /// Catalog query: SELECT id,cooldown_tag_id FROM skills. Present zero means a direct timer;
    /// missing entries remain unknown. Static catalog survives character/connection lifecycle.
    pub fn set_cooldown_catalog(&mut self, catalog: HashMap<u32, u32>) {
        self.cooldown_catalog = std::sync::Arc::new(catalog);
    }
    pub fn native_cooldown(&self, skill_id: u32, tags: &[u32]) -> Option<(Duration, Duration)> {
        self.native_cooldown_at(skill_id, tags, Instant::now())
    }
    /// Received-source timers only, including stop durations; no cast/database estimate.
    pub fn native_cooldown_at(&self, skill_id: u32, tags: &[u32], now: Instant) -> Option<(Duration, Duration)> {
        let candidates = tags
            .iter()
            .flat_map(|tag| self.native_tag_cooldowns.iter().filter(move |row| row.cooldown.id == *tag))
            .chain(self.native_skill_cooldowns.iter().filter(|row| row.cooldown.id == skill_id));
        let mut result = None;
        for row in candidates {
            let left = row.remaining_ms_at(now);
            if left != 0 && result.is_none_or(|(best, _)| left > best) {
                result = Some((left, row.cooldown.total_ms));
            }
        }
        result.map(|(left, total)| (Duration::from_millis(u64::from(left)), Duration::from_millis(u64::from(total))))
    }
    pub(super) fn clear_received_state(&mut self) {
        let last_frame_ms = self.native_last_frame_ms;
        let catalog = std::mem::take(&mut self.cooldown_catalog);
        let plot_gcd_catalog = std::mem::take(&mut self.plot_gcd_catalog);
        *self = Self {
            cooldown_catalog: catalog,
            plot_gcd_catalog,
            native_last_frame_ms: last_frame_ms,
            ..Self::default()
        };
    }
    pub(super) fn seed_native_timers(&mut self, timers: &crate::skill::SkillCooldowns, now: Instant) -> bool {
        fn append(rows: &mut Vec<NativeCooldown>, entries: &[Cooldown], now: Instant) -> bool {
            let before = rows.len();
            rows.extend(entries.iter().take(COOLDOWN_LIMIT.saturating_sub(before)).map(|&cooldown| NativeCooldown {
                cooldown,
                updated_at: now,
                origin: NativeCooldownOrigin::Snapshot,
                reset: false,
            }));
            rows.len() != before
        }
        let skills = append(&mut self.native_skill_cooldowns, &timers.skills, now);
        let tags = append(&mut self.native_tag_cooldowns, &timers.tags, now);
        skills || tags
    }
    pub(super) fn fold_stop_timer(&mut self, tl: u16, duration_ms: u32, plot: bool, now: Instant) -> bool {
        if duration_ms == 0 || self.native_gm_mode1 != Some(0) || self.obj_id == 0 {
            return false;
        }
        // Existing casting state is keyed by actor, not native timeline tree order. Refuse
        // ambiguity instead of choosing whichever actor HashMap happens to visit first.
        let mut matches = self.casting.iter().filter(|(_, cast)| cast.tl == tl);
        let Some((&actor, cast)) = matches.next() else { return false };
        if actor != self.obj_id || matches.next().is_some() {
            return false;
        }
        let Some(&tag) = self.cooldown_catalog.get(&cast.skill_id) else {
            return false;
        };
        let skill_id = cast.skill_id;
        // A genuine admitted stop replaces this app-only cast/DB observation, even when
        // native insertion keeps an already longer first row. Do not revive it on expiry.
        let estimate_removed = self.last_used.remove(&skill_id).is_some();
        let id = if tag == 0 { skill_id } else { tag };
        let rows = if tag == 0 {
            &mut self.native_skill_cooldowns
        } else {
            &mut self.native_tag_cooldowns
        };
        let origin = if plot {
            NativeCooldownOrigin::PlotStopped { tl }
        } else {
            NativeCooldownOrigin::CastingStopped { tl }
        };
        insert(rows, id, duration_ms, origin, now) || estimate_removed
    }
    pub(super) fn reset_native_timer(&mut self, skill_id: u32, tag_id: u32) {
        if let Some(row) = self.native_skill_cooldowns.iter_mut().find(|row| row.cooldown.id == skill_id) {
            row.reset = true;
        }
        if let Some(row) = self.native_tag_cooldowns.iter_mut().find(|row| row.cooldown.id == tag_id) {
            row.reset = true;
        }
    }
}
fn insert(rows: &mut Vec<NativeCooldown>, id: u32, duration_ms: u32, origin: NativeCooldownOrigin, now: Instant) -> bool {
    if let Some(row) = rows.iter_mut().find(|row| row.cooldown.id == id) {
        if duration_ms <= row.remaining_ms_at(now) {
            return false;
        }
        *row = NativeCooldown {
            cooldown: Cooldown {
                id,
                remaining_ms: duration_ms,
                total_ms: duration_ms,
            },
            updated_at: now,
            origin,
            reset: false,
        };
        return true;
    }
    if rows.len() >= COOLDOWN_LIMIT {
        rows.retain(|row| row.remaining_ms_at(now) > 0);
    }
    if rows.len() >= COOLDOWN_LIMIT {
        return false;
    }
    rows.push(NativeCooldown {
        cooldown: Cooldown {
            id,
            remaining_ms: duration_ms,
            total_ms: duration_ms,
        },
        updated_at: now,
        origin,
        reset: false,
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        buff::BuffCaster,
        session::PlayerSpawn,
        skill::{SkillCooldowns, SkillStarted},
        state::tests::unit,
    };
    fn row(id: u32, remaining_ms: u32, total_ms: u32, at: Instant) -> NativeCooldown {
        NativeCooldown {
            cooldown: Cooldown { id, remaining_ms, total_ms },
            updated_at: at,
            origin: NativeCooldownOrigin::Snapshot,
            reset: false,
        }
    }
    fn start(skill_id: u32, tl: u16, actor: u32) -> Event {
        Event::SkillStarted(SkillStarted {
            skill_id,
            tl,
            caster: BuffCaster { kind: 0, obj_id: actor },
            target: SkillTarget::unit(9),
            raw_caster:crate::SkillCasterData::Unit(actor),
            raw_target:crate::SkillTargetData::Unit(9),
            cast_ms: 100,
            base_cast_ms: 100,
            synergy: 0,
            result: 0,
            result_value: 0,
            raw_result:crate::SkillResult::default(),
        })
    }
    fn ready() -> Skills {
        let mut state = Skills::default();
        state.set_cooldown_catalog([(17, 0), (18, 77)].into());
        state.apply(&Event::PlayerSpawn(Box::new(PlayerSpawn {
            obj_id: 7,
            state: Default::default(),
            unit: unit(7),
        })));
        state.apply(&Event::UnitGmModeChanged(crate::unit_msg::UnitGmModeChanged {
            obj_id: 7,
            mode: 1,
            value: 0,
        }));
        state
    }
    fn values(rows: &[NativeCooldown], at: Instant) -> Vec<[u32; 3]> {
        rows.iter().map(|r| [r.cooldown.id, r.remaining_ms_at(at), r.cooldown.total_ms]).collect()
    }
    #[test]
    fn executed_x86_frame_oracles_saturate_unsigned_without_compacting_or_changing_totals() {
        // Original39899e50 executed in Unicorn, no mocked instructions/calls.
        for (delta, expected) in [(0,[0,1000,u32::MAX]),(1,[0,999,u32::MAX-1]),
            (999,[0,1,4294966296]),(1000,[0,0,4294966295]),
            (2147483647,[0,0,2147483648]),(u32::MAX,[0,0,0])] {
            let at=Instant::now();let mut state=Skills::default();
            state.native_last_frame_ms=1;
            let rows=vec![row(77,0,4000,at),row(78,1000,4001,at),row(79,u32::MAX,4002,at)];
            state.native_skill_cooldowns=rows.clone();state.native_tag_cooldowns=rows;
            // MAX delta needs a nonzero wrapped now (native now==0 has a special branch).
            if delta==u32::MAX {state.native_last_frame_ms=2;}
            let now=state.native_last_frame_ms.wrapping_add(delta);
            state.advance_native_frame(now);
            for rows in [&state.native_skill_cooldowns,&state.native_tag_cooldowns] {
                assert_eq!(rows.len(),3);
                for (i,r) in rows.iter().enumerate() {
                    assert_eq!(r.cooldown.remaining_ms,expected[i]);
                    assert_eq!(r.cooldown.total_ms,4000+i as u32);
                    assert_eq!(r.cooldown.id,77+i as u32);
                    assert_eq!(r.updated_at,at);
                }
            }
        }
    }
    #[test]
    fn frame_order_zero_wrap_and_lifecycle_preserve_process_clock() {
        let mut state=ready();let at=Instant::now();
        state.advance_native_frame(100);
        state.seed_native_timers(&SkillCooldowns{skills:vec![Cooldown{id:17,remaining_ms:1000,total_ms:2000}],tags:vec![]},at);
        assert_eq!(state.native_skill_cooldowns[0].remaining_ms_at(at+Duration::from_secs(999)),1000,"queries never project time");
        state.advance_native_frame(110);
        assert_eq!(state.native_skill_cooldowns[0].cooldown.remaining_ms,990,"fresh queued receipt loses full frame delta");
        assert!(!state.advance_native_frame(110));
        assert!(!state.advance_native_frame(0));
        assert_eq!(state.native_last_frame_ms,0);
        state.advance_native_frame(5);
        assert_eq!(state.native_skill_cooldowns[0].cooldown.remaining_ms,985);
        state.native_last_frame_ms=u32::MAX-2;state.advance_native_frame(3);
        assert_eq!(state.native_skill_cooldowns[0].cooldown.remaining_ms,979);
        for e in [Event::CharacterSelected(Box::default()),Event::LeftWorld{target:0},Event::Disconnected{message:String::new()},Event::PlayerSpawn(Box::new(PlayerSpawn{obj_id:7,state:Default::default(),unit:unit(7)}))] {
            state.apply(&e);assert_eq!(state.native_last_frame_ms,3);
            assert!(state.native_skill_cooldowns.is_empty());
        }
    }
    #[test]
    fn original_x86_oracle_first_matching_row_extension_and_expired_reuse() {
        // Executed original39899f90 outputs, UI native-cooldown-producer-oracle-20261018.json.
        let at = Instant::now();
        let origin = NativeCooldownOrigin::CastingStopped { tl: 3 };
        let mut rows = vec![row(17, 5000, 10000, at), row(17, 9000, 11000, at)];
        assert!(insert(&mut rows, 17, 7000, origin, at));
        assert_eq!(values(&rows, at), vec![[17, 7000, 7000], [17, 9000, 11000]]);
        rows = vec![row(17, 8000, 10000, at), row(17, 9000, 11000, at)];
        assert!(!insert(&mut rows, 17, 7000, origin, at));
        assert_eq!(values(&rows, at), vec![[17, 8000, 10000], [17, 9000, 11000]]);
        rows = vec![row(17, 0, 10000, at), row(17, 9000, 11000, at)];
        assert!(insert(&mut rows, 17, 250, origin, at));
        assert_eq!(values(&rows, at), vec![[17, 250, 250], [17, 9000, 11000]]);
        // Native compares whole milliseconds: submillisecond time must not create an extension.
        assert!(!insert(&mut rows, 17, 250, origin, at + Duration::from_nanos(1)));
    }
    #[test]
    fn original_x86_oracle_capacity_compaction_preserves_survivor_order() {
        let at = Instant::now();
        let origin = NativeCooldownOrigin::CastingStopped { tl: 3 };
        let mut rows = (100..250)
            .map(|id| row(id, if [100, 150, 200].contains(&id) { 1000 } else { 0 }, 2000, at))
            .collect::<Vec<_>>();
        assert!(insert(&mut rows, 17, 250, origin, at));
        assert_eq!(values(&rows, at), vec![[100, 1000, 2000], [150, 1000, 2000], [200, 1000, 2000], [17, 250, 250]]);
        rows = (100..250).map(|id| row(id, 1000, 2000, at)).collect();
        let before = rows.clone();
        assert!(!insert(&mut rows, 17, 250, origin, at));
        assert_eq!(rows, before);
        rows[0].reset = true;
        assert!(insert(&mut rows, 17, 250, origin, at));
        assert_eq!(rows.len(), 150);
        assert_eq!(rows[0].cooldown.id, 101);
        assert_eq!(rows[149].cooldown.id, 17);
    }
    #[test]
    fn stop_compares_frame_remainder_not_wall_clock_and_receipts_remain_unchanged() {
        let at=Instant::now();let mut state=ready();state.advance_native_frame(100);
        state.apply(&Event::Cooldowns(SkillCooldowns{skills:vec![Cooldown{id:17,remaining_ms:1000,total_ms:2000}],tags:vec![]}));
        let receipts=state.received_skill_cooldowns.clone();
        state.apply(&start(17,3,7));
        assert!(state.fold_stop_timer(3,900,false,at+Duration::from_secs(100)),"only estimate invalidation, existing1000 wins");
        assert_eq!(state.native_skill_cooldowns[0].cooldown.remaining_ms,1000);
        state.advance_native_frame(300);
        state.apply(&start(17,4,7));
        assert!(state.fold_stop_timer(4,900,false,at));
        assert_eq!(state.native_skill_cooldowns[0].cooldown,Cooldown{id:17,remaining_ms:900,total_ms:900});
        assert_eq!(state.received_skill_cooldowns,receipts);
    }
    #[test]
    fn actual_cast_stop_inserts_before_cast_removal_and_preserves_raw_snapshot_history() {
        let mut state = ready();
        state.apply(&Event::Cooldowns(SkillCooldowns {
            skills: vec![Cooldown {
                id: 17,
                remaining_ms: 0,
                total_ms: 10000,
            }],
            tags: vec![],
        }));
        let raw = state.received_skill_cooldowns.clone();
        state.apply(&start(17, 3, 7));
        assert!(state.apply(&Event::CastingStopped { tl: 3, duration_ms: 7000 }));
        assert!(state.casting.is_empty());
        assert_eq!(state.received_skill_cooldowns, raw);
        let timer = state.native_skill_cooldowns[0];
        assert_eq!(timer.origin, NativeCooldownOrigin::CastingStopped { tl: 3 });
        assert_eq!(
            state.native_cooldown_at(17, &[], timer.updated_at),
            Some((Duration::from_millis(7000), Duration::from_millis(7000)))
        );
        assert_eq!(state.received_cooldown_at(17, &[], timer.updated_at), None);
        assert!(!state.apply(&Event::CastingStopped { tl: 3, duration_ms: 9000 }));
        assert_eq!(state.native_skill_cooldowns[0], timer, "duplicate stop with no tracked timeline cannot extend");
    }
    #[test]
    fn missing_ambiguous_foreign_unknown_catalog_and_suppressed_stops_never_invent_timers() {
        for case in 0..6 {
            let mut state = ready();
            state.apply(&start(17, 3, 7));
            match case {
                0 => state.native_gm_mode1 = None,
                1 => {
                    state.apply(&Event::UnitGmModeChanged(crate::unit_msg::UnitGmModeChanged {
                        obj_id: 7,
                        mode: 1,
                        value: 255,
                    }));
                }
                2 => {
                    state.apply(&start(18, 3, 8));
                } // Duplicate TL is deliberately ambiguous.
                3 => {
                    state.casting.remove(&7);
                    state.apply(&start(17, 3, 8));
                }
                4 => state.set_cooldown_catalog(HashMap::new()),
                _ => {}
            }
            state.apply(&Event::CastingStopped {
                tl: 3,
                duration_ms: if case == 5 { 0 } else { 7000 },
            });
            assert!(state.native_skill_cooldowns.is_empty() && state.native_tag_cooldowns.is_empty(), "case{case}");
            assert!(state.casting.is_empty(), "existing cast teardown remains compatible");
            assert!(state.last_used.contains_key(&17), "unadmitted stop keeps prior observation, case{case}");
        }
    }
    #[test]
    fn plot_stop_routes_primary_tag_and_ordered_query_keeps_received_totals() {
        let mut state = ready();
        state.apply(&Event::PlotEvent(crate::skill::PlotEvent {
            tl: 4,
            event_id: 2,
            skill_id: 18,
            caster_obj_id: 7,
            target_obj_id: 9,
            caster_pos: None,
            target_pos: None,
            casting_ms: 10,
        }));
        state.apply(&Event::PlotStopped {
            tl: 4,
            duration_ms: 7000,
            last_event: 255,
            channeling: true,
        });
        assert!(state.native_skill_cooldowns.is_empty());
        let timer = state.native_tag_cooldowns[0];
        assert_eq!(timer.cooldown.id, 77);
        assert_eq!(timer.origin, NativeCooldownOrigin::PlotStopped { tl: 4 });
        state.native_tag_cooldowns.push(row(78, 7000, 9000, timer.updated_at));
        state.native_skill_cooldowns.push(row(18, 7000, 10000, timer.updated_at));
        assert_eq!(
            state.native_cooldown_at(18, &[78, 77], timer.updated_at),
            Some((Duration::from_millis(7000), Duration::from_millis(9000)))
        );
        assert_eq!(
            state.native_cooldown_at(18, &[77, 78], timer.updated_at),
            Some((Duration::from_millis(7000), Duration::from_millis(7000)))
        );
    }
    #[test]
    fn genuine_resets_target_first_runtime_duplicate_and_catalog_survives_lifecycle() {
        let mut state = ready();
        let at = Instant::now();
        state.native_tag_cooldowns = vec![row(77, 7000, 8000, at), row(77, 9000, 11000, at)];
        state.apply(&Event::SkillCooldownReset {
            obj_id: 8,
            skill_id: 0,
            tag_id: 77,
            gcd: false,
        });
        assert!(!state.native_tag_cooldowns[0].reset);
        state.apply(&Event::SkillCooldownReset {
            obj_id: 7,
            skill_id: 0,
            tag_id: 77,
            gcd: false,
        });
        assert!(state.native_tag_cooldowns[0].reset && !state.native_tag_cooldowns[1].reset);
        assert_eq!(
            state.native_cooldown_at(18, &[77], at),
            Some((Duration::from_millis(9000), Duration::from_millis(11000)))
        );
        for event in [
            Event::CharacterSelected(Box::default()),
            Event::LeftWorld { target: 0 },
            Event::Disconnected { message: String::new() },
            Event::Kicked {
                reason: 0,
                message: String::new(),
            },
        ] {
            state.apply(&event);
            assert_eq!(state.cooldown_catalog.get(&18), Some(&77));
            assert!(state.native_tag_cooldowns.is_empty() && state.native_skill_cooldowns.is_empty());
            assert_eq!(state.native_gm_mode1, None);
        }
        let mut spawn = unit(7);
        spawn.gm_modes[1] = 2;
        state.apply(&Event::PlayerSpawn(Box::new(PlayerSpawn {
            obj_id: 7,
            state: Default::default(),
            unit: spawn,
        })));
        assert_eq!(state.native_gm_mode1, None, "the legacy PISH word is not effective GM hydration");
        state.apply(&Event::UnitGmModeChanged(crate::unit_msg::UnitGmModeChanged { obj_id: 7, mode: 1, value: 2 }));
        assert_eq!(state.native_gm_mode1, Some(2));
        assert_eq!(state.cooldown_catalog.get(&17), Some(&0));
    }
    #[test]
    fn snapshot_seeds_both_views_but_runtime_compaction_never_rewrites_raw_receipts() {
        let mut state = ready();
        let snapshot = SkillCooldowns {
            skills: (100..250)
                .map(|id| Cooldown {
                    id,
                    remaining_ms: 0,
                    total_ms: 2000,
                })
                .collect(),
            tags: vec![],
        };
        state.apply(&Event::Cooldowns(snapshot));
        let raw = state.received_skill_cooldowns.clone();
        assert_eq!(state.native_skill_cooldowns.len(), 150);
        state.apply(&start(17, 3, 7));
        state.apply(&Event::CastingStopped { tl: 3, duration_ms: 250 });
        assert_eq!(state.native_skill_cooldowns.len(), 1);
        assert_eq!(state.native_skill_cooldowns[0].cooldown.id, 17);
        assert_eq!(state.received_skill_cooldowns, raw);
        // Raw history remains at capacity while the actual manager now has room for new snapshots.
        state.apply(&Event::Cooldowns(SkillCooldowns {
            skills: vec![Cooldown {
                id: 8,
                remaining_ms: 123,
                total_ms: 456,
            }],
            tags: vec![],
        }));
        assert_eq!(state.received_skill_cooldowns, raw);
        assert_eq!(state.native_skill_cooldowns.len(), 2);
        assert_eq!(state.native_skill_cooldowns[1].origin, NativeCooldownOrigin::Snapshot);
    }
    #[test]
    fn world_state_hydration_and_own_removal_guard_stop_flow_without_started_timer_estimates() {
        let mut world = crate::state::WorldState::default();
        world.skills.set_cooldown_catalog([(17, 0)].into());
        world.apply(&Event::PlayerSpawn(Box::new(PlayerSpawn {
            obj_id: 7,
            state: Default::default(),
            unit: unit(7),
        })));
        world.apply(&start(17, 3, 7));
        assert!(world.skills.native_skill_cooldowns.is_empty());
        world.apply(&Event::UnitsRemoved(vec![7]));
        assert_eq!(world.skills.native_gm_mode1, None);
        world.apply(&start(17, 4, 7));
        world.apply(&Event::CastingStopped { tl: 4, duration_ms: 7000 });
        assert!(world.skills.native_skill_cooldowns.is_empty());
        world.apply(&Event::PlayerSpawn(Box::new(PlayerSpawn {
            obj_id: 7,
            state: Default::default(),
            unit: unit(7),
        })));
        world.apply(&start(17, 5, 7));
        world.apply(&Event::UnitGmModeChanged(crate::unit_msg::UnitGmModeChanged { obj_id: 7, mode: 1, value: 0 }));
        world.apply(&Event::CastingStopped { tl: 5, duration_ms: 7000 });
        let cloned = world.clone();
        assert_eq!(cloned.skills.native_skill_cooldowns[0].cooldown.remaining_ms, 7000);
        assert_eq!(cloned.skills.native_skill_cooldowns[0].origin, NativeCooldownOrigin::CastingStopped { tl: 5 });
    }
    #[test]
    fn admitted_stop_invalidates_only_resolved_estimate_even_when_longer_timer_wins() {
        for (skill,tag) in [(17,0),(18,77)] {
            let mut state=ready();state.apply(&start(skill,3,7));
            let at=Instant::now();state.last_used.insert(999,at);
            let timer=row(if tag==0 {skill}else{tag},8000,10000,at);
            if tag==0 {state.native_skill_cooldowns.push(timer);}else{state.native_tag_cooldowns.push(timer);}
            assert!(state.cooldown_with_total_at(skill,tag,60000,at).is_some());
            assert!(state.fold_stop_timer(3,1000,false,at),"estimate removal is a state change despite insertion no-op");
            assert!(!state.last_used.contains_key(&skill));assert!(state.last_used.contains_key(&999));
            assert_eq!(if tag==0 {state.native_skill_cooldowns[0]}else{state.native_tag_cooldowns[0]},timer);
            assert_eq!(state.native_cooldown_at(skill,&[tag],at),Some((Duration::from_millis(8000),Duration::from_millis(10000))));
            state.advance_native_frame(8001);
            let expired=at+Duration::from_millis(8001);
            assert!(state.native_cooldown_at(skill,&[tag],expired).is_none());
            assert!(state.cooldown_with_total_at(skill,tag,60000,expired).is_none(),"expired real timer must not reveal stale estimate");
            state.apply(&start(skill,4,7));
            assert!(state.last_used.contains_key(&skill));
            assert!(state.cooldown_with_total_at(skill,tag,60000,Instant::now()).is_some(),"later genuine Started can create a new observation");
        }
    }

    #[test]
    fn static_catalog_clone_and_lifecycle_share_one_allocation() {
        let mut state=ready();let snapshot=state.clone();
        assert!(std::sync::Arc::ptr_eq(&state.cooldown_catalog,&snapshot.cooldown_catalog));
        for event in [Event::PlayerSpawn(Box::new(PlayerSpawn{obj_id:7,state:Default::default(),unit:unit(7)})),Event::CharacterSelected(Box::default()),Event::Disconnected{message:String::new()}] {
            state.apply(&event);
            assert!(std::sync::Arc::ptr_eq(&state.cooldown_catalog,&snapshot.cooldown_catalog));
            assert_eq!(state.cooldown_catalog.get(&18),Some(&77));
        }
        state.set_cooldown_catalog([(17,0),(18,77)].into());
        assert!(!std::sync::Arc::ptr_eq(&state.cooldown_catalog,&snapshot.cooldown_catalog));
        assert_eq!(state.cooldown_catalog,snapshot.cooldown_catalog,"value equality does not require pointer identity");
    }

    #[test]
    fn spawn_pish_bytes_never_admit_stop_without_matching_effective_mode_receipt() {
        // Executed original hydration + SC203 with distinct actor/subobject bases:
        // native-unit-actor-base-oracle.json. PISH word lands at raw actor+3b00;
        // SC203 mode1 and the cooldown guard address raw actor+3af9 instead.
        for raw in [0, 2, 255] {
            let mut state = ready();
            let mut spawn = unit(7);
            spawn.gm_modes[1] = raw;
            state.apply(&Event::PlayerSpawn(Box::new(PlayerSpawn { obj_id: 7, state: Default::default(), unit: spawn })));
            assert_eq!(state.native_gm_mode1, None);
            state.apply(&start(17, 3, 7));
            state.apply(&Event::CastingStopped { tl: 3, duration_ms: 1000 });
            assert!(state.native_skill_cooldowns.is_empty());
            assert!(state.last_used.contains_key(&17), "unadmitted stop does not invalidate estimates");
            for change in [
                crate::unit_msg::UnitGmModeChanged { obj_id: 8, mode: 1, value: 0 },
                crate::unit_msg::UnitGmModeChanged { obj_id: 7, mode: 0, value: 0 },
            ] {
                state.apply(&Event::UnitGmModeChanged(change));
            }
            assert_eq!(state.native_gm_mode1, None);
            state.apply(&Event::UnitGmModeChanged(crate::unit_msg::UnitGmModeChanged { obj_id: 7, mode: 1, value: 0 }));
            state.apply(&start(17, 4, 7));
            state.apply(&Event::CastingStopped { tl: 4, duration_ms: 1000 });
            assert_eq!(state.native_skill_cooldowns[0].cooldown.remaining_ms, 1000);
            assert!(!state.last_used.contains_key(&17));
        }
    }

    #[test]
    fn removed_actor_rejects_late_mode_until_genuine_fresh_spawn() {
        let mut state = ready();
        assert!(state.apply(&Event::UnitsRemoved(vec![7])));
        assert_eq!(state.native_gm_mode1, None);
        assert!(!state.apply(&Event::UnitGmModeChanged(crate::unit_msg::UnitGmModeChanged { obj_id: 7, mode: 1, value: 0 })));
        assert_eq!(state.native_gm_mode1, None);
        state.apply(&start(17, 3, 7));
        state.apply(&Event::CastingStopped { tl: 3, duration_ms: 1000 });
        assert!(state.native_skill_cooldowns.is_empty());
        state.apply(&Event::PlayerSpawn(Box::new(PlayerSpawn { obj_id: 7, state: Default::default(), unit: unit(7) })));
        assert_eq!(state.native_gm_mode1, None);
        state.apply(&Event::UnitGmModeChanged(crate::unit_msg::UnitGmModeChanged { obj_id: 7, mode: 1, value: 0 }));
        state.apply(&start(17, 4, 7));
        state.apply(&Event::CastingStopped { tl: 4, duration_ms: 1000 });
        assert_eq!(state.native_skill_cooldowns[0].cooldown.remaining_ms, 1000);
    }

}
