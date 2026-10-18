//! Native low-durability edge tracker. Deliberately replays only its handled task kinds.
use super::GameData;
use aa_net::items::{ItemTask, ItemTasks, action};
use aa_net::model::{Item, ItemDetail};
use aa_net::state::Inventory;
use std::collections::HashMap;

#[derive(Default)]
pub(super) struct Wear {
    pub(super) active: bool,
    values: HashMap<u64, (u8, u8)>,
}

fn low(current: u8, maximum: u8) -> bool {
    maximum != 0 && f32::from(current) / f32::from(maximum) <= 0.25f32
}

fn pair(game: &GameData, item: &Item) -> Option<(u8, u8)> {
    let ItemDetail::Equip(detail) = &item.detail else { return None };
    let (maximum, _) = game.tips.items.durability(item.template_id, item.grade)?;
    Some((detail.durability, maximum))
}

impl Wear {
    pub(super) fn hydrate(&mut self, game: &GameData, inventory: &Inventory) {
        self.values.clear();
        for item in inventory.equipment.slots.iter().take(28).chain(inventory.bag.iter()).flatten() {
            if let Some(pair) = pair(game, item) {
                self.values.insert(item.id, pair);
            }
        }
    }

    fn insert(&mut self, uid: u64, pair: (u8, u8), added: &mut bool) {
        self.values.insert(uid, pair);
        *added |= low(pair.0, pair.1);
    }

    fn remove(&mut self, uid: u64, removed: &mut bool) {
        if let Some((current, maximum)) = self.values.remove(&uid) {
            *removed |= low(current, maximum);
        }
    }

    fn update(&mut self, uid: u64, current: u8, added: &mut bool, removed: &mut bool) {
        if let Some((previous, maximum)) = self.values.get_mut(&uid) {
            let was = low(*previous, *maximum);
            let now = low(current, *maximum);
            *added |= !was && now;
            *removed |= was && !now;
            *previous = current;
        }
    }

    pub(super) fn batch(&mut self, game: &GameData, tasks: &ItemTasks) -> Option<(bool, bool)> {
        let (mut added, mut removed) = (false, false);
        for task in &tasks.tasks {
            match task {
                ItemTask::Add {
                    action: action::CREATE | action::TAKE,
                    container: 1 | 2 | 3,
                    item,
                    ..
                } => {
                    if let Some(pair) = pair(game, item) {
                        self.insert(item.id, pair, &mut added);
                    }
                }
                ItemTask::Remove { item_id, .. } => self.remove(*item_id, &mut removed),
                ItemTask::Update {
                    item_id,
                    detail: ItemDetail::Equip(detail),
                    ..
                } => self.update(*item_id, detail.durability, &mut added, &mut removed),
                _ => {}
            }
        }
        (added || removed).then_some((added, removed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn threshold_is_inclusive_sticky_and_uses_stored_maximum() {
        let mut wear = Wear::default();
        let (mut added, mut removed) = (false, false);
        wear.insert(9_007_199_254_740_993, (9, 32), &mut added);
        assert!(!added);
        wear.update(9_007_199_254_740_993, 8, &mut added, &mut removed);
        assert_eq!((added, removed), (true, false));
        wear.update(9_007_199_254_740_993, 0, &mut added, &mut removed);
        wear.update(9_007_199_254_740_993, 9, &mut added, &mut removed);
        assert_eq!((added, removed), (true, true), "crossing both ways in one batch retains both edges");
        let (mut added, mut removed) = (false, false);
        wear.update(999, 0, &mut added, &mut removed);
        wear.remove(999, &mut removed);
        assert_eq!((added, removed), (false, false));
        wear.insert(42, (1, 4), &mut added);
        assert!(added);
        added = false;
        wear.insert(42, (1, 4), &mut added);
        assert!(added, "native duplicate low insertion reports added again");
        wear.remove(42, &mut removed);
        assert!(removed);
    }

    #[test]
    fn received_updates_preserve_native_ignored_actions_and_batch_edges() {
        let game = GameData::default();
        let mut wear = Wear::default();
        let mut added = false;
        wear.insert(47, (1, 4), &mut added);
        let ignored = ItemTasks {
            tasks: vec![
                ItemTask::Grade {
                    container: 1,
                    slot: 0,
                    item_id: 47,
                    grade: 7,
                },
                ItemTask::RemoveCrafting { item_id: 47 },
                ItemTask::RemoveSlot {
                    container: 1,
                    slot: 0,
                    item_id: 47,
                },
                ItemTask::Move {
                    from_container: 1,
                    from_slot: 0,
                    from_item_id: 47,
                    to_container: 3,
                    to_slot: 0,
                    to_item_id: 0,
                },
            ],
            force_remove: vec![47],
            ..Default::default()
        };
        assert_eq!(wear.batch(&game, &ignored), None);
        assert_eq!(wear.values.get(&47), Some(&(1, 4)));
        let detail = |durability| ItemTask::Update {
            container: 1,
            slot: 0,
            item_id: 47,
            detail: ItemDetail::Equip(aa_net::model::EquipDetail {
                durability,
                ..Default::default()
            }),
            detail_payload: None,
        };
        assert_eq!(
            wear.batch(
                &game,
                &ItemTasks {
                    tasks: vec![detail(4), detail(0)],
                    ..Default::default()
                }
            ),
            Some((true, true))
        );
        assert_eq!(
            wear.batch(
                &game,
                &ItemTasks {
                    tasks: vec![detail(1)],
                    ..Default::default()
                }
            ),
            None,
            "staying low emits no new event"
        );
        assert_eq!(
            wear.batch(
                &game,
                &ItemTasks {
                    tasks: vec![ItemTask::Remove {
                        container: 1,
                        slot: 0,
                        item_id: 47,
                        template_id: 1
                    }],
                    ..Default::default()
                }
            ),
            Some((false, true))
        );
    }
}
