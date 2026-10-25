//! Displayed context and captured indices for the original shared action bar.
//! This does not establish which world context qualifies for publication.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Source {
    Doodad {
        obj_id: u32,
        phase: u32,
        generation: u64,
    },
    #[allow(dead_code)] // Native NPC context providers are not yet all qualified.
    Npc { obj_id: u32, template: u32 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Shown {
    pub(super) source: Source,
    pub(super) skills: Vec<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Pick {
    pub(super) source: Source,
    pub(super) index: usize,
    pub(super) skill: u32,
    revision: u64,
}

#[derive(Default)]
pub(super) struct View {
    shown: Option<Shown>,
    revision: u64,
    requested: Vec<Pick>,
}

impl View {
    pub(super) fn shown(&self) -> Option<&Shown> {
        self.shown.as_ref()
    }

    pub(super) fn replace(&mut self, shown: Option<Shown>) -> bool {
        if self.shown == shown {
            return false;
        }
        self.revision = self.revision.wrapping_add(1);
        self.shown = shown;
        true
    }

    #[allow(dead_code)] // Owner-qualified retirement is also used by the NPC bridge.
    pub(super) fn withdraw(&mut self, source: Source) -> bool {
        if self.shown.as_ref().is_some_and(|s| s.source == source) {
            return self.replace(None);
        }
        false
    }

    pub(super) fn count(&self) -> usize {
        self.shown.as_ref().map_or(0, |s| s.skills.len())
    }

    pub(super) fn skill(&self, index: usize) -> Option<u32> {
        self.shown
            .as_ref()?
            .skills
            .get(index.checked_sub(1)?)
            .copied()
            .filter(|skill| *skill != 0)
    }

    pub(super) fn request(&mut self, index: usize) {
        if let Some(skill) = self.skill(index) {
            let pick = Pick {
                source: self.shown.as_ref().unwrap().source,
                index,
                skill,
                revision: self.revision,
            };
            // Original buttons bind both OnButtonUp and OnClick. A physical
            // release must not become two deferred submissions of that slot.
            if !self.requested.contains(&pick) {
                self.requested.push(pick);
            }
        }
    }

    fn current(&self, pick: &Pick) -> bool {
        pick.revision == self.revision
            && self.shown.as_ref().is_some_and(|s| s.source == pick.source)
            && self.skill(pick.index) == Some(pick.skill)
    }

    pub(super) fn take_requested(&mut self) -> Vec<Pick> {
        std::mem::take(&mut self.requested)
            .into_iter()
            .filter(|pick| self.current(pick))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn npc(id: u32, skills: &[u32]) -> Shown {
        Shown {
            source: Source::Npc {
                obj_id: id,
                template: 42,
            },
            skills: skills.to_vec(),
        }
    }

    #[test]
    fn dynamic_indices_preserve_zero_and_duplicate_slots() {
        let mut bar = View::default();
        bar.replace(Some(npc(10, &[0, 12087, 12087, 12083])));
        assert_eq!(bar.count(), 4);
        assert_eq!(bar.skill(0), None);
        assert_eq!(bar.skill(1), None);
        assert_eq!(bar.skill(2), Some(12087));
        assert_eq!(bar.skill(3), Some(12087));
        assert_eq!(bar.skill(4), Some(12083));
        assert_eq!(bar.skill(5), None);
        for i in 0..=5 {
            bar.request(i);
        }
        let picks = bar.take_requested();
        assert_eq!(
            picks.iter().map(|p| (p.index, p.skill)).collect::<Vec<_>>(),
            [(2, 12087), (3, 12087), (4, 12083)]
        );
    }

    #[test]
    fn a_queued_index_cannot_move_to_a_new_source_or_revision() {
        let mut bar = View::default();
        bar.replace(Some(npc(10, &[12087, 12083])));
        bar.request(2);
        bar.replace(Some(npc(11, &[12087, 12083])));
        assert!(bar.take_requested().is_empty());
        bar.request(2);
        bar.replace(Some(npc(11, &[12083, 12087])));
        assert!(bar.take_requested().is_empty());
        bar.request(2);
        bar.replace(None);
        bar.replace(Some(npc(11, &[12083, 12087])));
        assert!(bar.take_requested().is_empty());
    }

    #[test]
    fn stale_owner_cannot_hide_a_different_source() {
        let mut bar = View::default();
        let a = npc(10, &[12087]);
        let b = Shown {
            source: Source::Doodad {
                obj_id: 10,
                phase: 3,
                generation: 7,
            },
            skills: vec![123],
        };
        bar.replace(Some(b.clone()));
        assert!(!bar.withdraw(a.source));
        assert_eq!(bar.shown(), Some(&b));
        bar.request(1);
        let mut new_phase = b.clone();
        new_phase.source = Source::Doodad {
            obj_id: 10,
            phase: 4,
            generation: 7,
        };
        bar.replace(Some(new_phase));
        assert!(bar.take_requested().is_empty());
        assert!(!bar.withdraw(b.source));
    }

    #[test]
    fn a_recreated_doodad_cannot_reuse_an_old_pick_with_identical_phase_and_skills() {
        let mut bar = View::default();
        let mut shown = Shown {
            source: Source::Doodad {
                obj_id: 10,
                phase: 3,
                generation: 7,
            },
            skills: vec![123],
        };
        bar.replace(Some(shown.clone()));
        bar.request(1);
        shown.source = Source::Doodad {
            obj_id: 10,
            phase: 3,
            generation: 8,
        };
        bar.replace(Some(shown));
        assert!(bar.take_requested().is_empty());
    }

    #[test]
    #[ignore = "loads original UI/pak and read-only game catalog"]
    fn original_dynamic_bar_uses_displayed_indices_and_real_pointer_delivery()
    -> Result<(), Box<dyn std::error::Error>> {
        let (rt, shared, path, _) = crate::hud::expansion::fixture::runtime(1280., 720.)?;
        shared
            .borrow_mut()
            .dynamic_actions
            .replace(Some(npc(10, &[0, 12087, 12083, 12087])));
        rt.lua.load("ADDON:ShowContent(UIC_BAG,false)").exec()?;
        rt.fire_event("DYNAMIC_ACTION_BAR_SHOW", ("npc", 10u32));
        aa_ui::lua::tick(&rt.lua, 0.);
        rt.lua.load(r#"
            assert(dynamic_action_bar:IsVisible())
            assert(X2:GetDynamicActionCount()==4)
            assert(select('#',X2:GetDynamicActionSkill(1))==0)
            assert(select('#',X2:GetDynamicActionSkill(5))==0)
            assert(not dynamic_action_bar.button[1]:IsVisible())
            assert(dynamic_action_bar.button[3]:IsVisible() and dynamic_action_bar.button[3].index==3)
        "#).exec()?;
        crate::hud::expansion::fixture::click(&rt, "dynamic_action_bar.button[3]")?;
        let picks = shared.borrow_mut().dynamic_actions.take_requested();
        assert_eq!(picks.len(), 1, "ButtonUp and Click must submit only once");
        assert_eq!(
            (picks[0].source, picks[0].index, picks[0].skill),
            (
                Source::Npc {
                    obj_id: 10,
                    template: 42
                },
                3,
                12083
            )
        );
        // A retained Lua callback queues against A; B replaces the displayed
        // context before the world consumer runs. It must never receive A's pick.
        rt.lua.load("X2:ExecuteDynamicAction(2)").exec()?;
        shared
            .borrow_mut()
            .dynamic_actions
            .replace(Some(npc(11, &[12087, 12083])));
        assert!(
            shared
                .borrow_mut()
                .dynamic_actions
                .take_requested()
                .is_empty()
        );
        shared.borrow_mut().dynamic_actions.replace(None);
        rt.fire_event("DYNAMIC_ACTION_BAR_HIDE", ());
        rt.lua.load("assert(not dynamic_action_bar:IsVisible());for i=1,10 do assert(dynamic_action_bar.button[i].index==0) end").exec()?;
        assert!(rt.errors.is_empty(), "{:?}", rt.errors);
        drop(rt);
        std::fs::remove_file(path)?;
        Ok(())
    }
}
