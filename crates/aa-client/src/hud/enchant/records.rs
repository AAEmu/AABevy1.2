//! Known native pre-row records. Never reconstruct these from the legacy reducer.
use aa_net::{
    Event,
    item_task_projection::{
        NativeItemTaskInventory, NativeItemTaskLocation, NativeItemTaskMutation, NativeItemTaskStorage, lookup_native_item_task,
        lookup_native_item_task_move, project_native_item_task, project_native_item_task_move,
    },
    items::{ItemTask, ItemTasks},
    model::{Equipment, Item},
};

#[derive(Default)]
pub(super) struct Records {
    owner: Option<u32>,
    actor: u32,
    equipment: Vec<Option<Item>>,
    bag: Vec<Option<Item>>,
    bank: Vec<Option<Item>>,
    bag_extent: Option<u8>,
    bank_extent: Option<u8>,
    lost: bool,
}

/// Returned only after all mutations of this individual row are visible.
pub(super) struct RowOutput {
    pub admitted: bool,
    pub item: Option<Item>,
}

fn empty_equipment() -> Item {
    // Fresh SC040/SC069/SC094 Item ctor3901cb10, unlike SC04c's whole-block memset.
    Item {
        world_id: 255,
        ..Item::default()
    }
}
fn equipment_snapshot(equipment: &Equipment) -> Vec<Option<Item>> {
    equipment
        .slots
        .iter()
        .enumerate()
        .map(|(slot, item)| {
            Some(if (19..=25).contains(&slot) {
                // SC040 and SC069 Character kind0 body slots use3924cdb0 even
                // for template0. The latter's mode2 exception is Slave kind2.
                Item {
                    template_id: item.as_ref().map_or(0, |i| i.template_id),
                    flags: 1,
                    count: 1,
                    ..empty_equipment()
                }
            } else {
                item.clone().unwrap_or_else(empty_equipment)
            })
        })
        .collect()
}
fn extent(slots: u32) -> u8 {
    (slots as u8 / 10) * 10
}
impl Records {
    fn selected(&mut self, owner: u32, equipment: &Equipment) {
        *self = Self {
            owner: Some(owner),
            equipment: equipment_snapshot(equipment),
            bag: vec![None; 130],
            bank: vec![None; 130],
            ..Self::default()
        };
    }
    fn lose(&mut self) {
        self.lost = true;
        self.equipment.fill(None);
        self.bag.fill(None);
        self.bank.fill(None);
    }
    /// Snapshot/lifecycle observations only. ItemsChanged rows must be applied by
    /// the caller one at a time, with its controller callback between rows.
    pub fn observe(&mut self, event: &Event) {
        match event {
            Event::CharacterSelected(selected) => self.selected(selected.character.id, &selected.character.equipment),
            Event::PlayerSpawn(spawn) => {
                if self.owner != Some(spawn.character().id) {
                    self.selected(spawn.character().id, &spawn.state.character.equipment);
                }
                self.actor = spawn.obj_id;
                self.equipment = equipment_snapshot(&spawn.unit.equipment);
            }
            Event::InventoryInit { bag_slots, bank_slots } => {
                self.bag_extent = Some(extent(*bag_slots));
                self.bank_extent = Some(extent(*bank_slots));
                // Explicit app certainty boundary: a new Init needs fresh rows.
                self.bag.fill(None);
                self.bank.fill(None);
            }
            Event::InventoryExpanded { container: 2, slots } => self.bag_extent = Some(extent(u32::from(*slots))),
            Event::InventoryExpanded { container: 3, slots } => self.bank_extent = Some(extent(u32::from(*slots))),
            Event::InventoryChunk(chunk) => {
                if chunk.truncated
                    || chunk.chunks > 5
                    || chunk.items.len() != usize::from(chunk.chunks) * 10
                    || chunk.first_slot() + chunk.items.len() > 130
                {
                    self.lose();
                    return;
                }
                let rows = match chunk.container {
                    2 => &mut self.bag,
                    3 => &mut self.bank,
                    _ => return,
                };
                for (offset, item) in chunk.items.iter().enumerate() {
                    let Some(row) = rows.get_mut(chunk.first_slot() + offset) else {
                        self.lose();
                        return;
                    };
                    // Actual packet ctor3961e690 memsets the whole block to0;
                    // a wire template0 returns before touching those other fields.
                    *row = Some(item.clone().unwrap_or_default());
                }
            }
            Event::EquipmentChanged { obj_id, items } if self.actor != 0 && *obj_id == self.actor => {
                if items.len() > 28 || items.iter().any(|(slot, _)| *slot >= 28) {
                    self.lose();
                    return;
                }
                for (slot, item) in items {
                    // SC094 serializes full Items even for body slots. Duplicates
                    // replace in received order; its ctor empty has worldFF.
                    if let Some(row) = self.equipment.get_mut(usize::from(*slot)) {
                        *row = Some(item.clone().unwrap_or_else(empty_equipment));
                    }
                }
            }
            Event::ItemsChanged(changes) if changes.truncated => self.lose(),
            Event::UnitsRemoved(ids) if self.actor != 0 && ids.contains(&self.actor) => {
                self.lose();
                self.actor = 0;
            }
            Event::GameDecodeFailed { .. } | Event::UnhandledGamePacket { level: 4, len: 1.., .. } => self.lose(),
            event if super::retires(event) => *self = Self::default(),
            _ => {}
        }
    }
    fn project(&self, task: &ItemTask) -> Option<(Vec<NativeItemTaskMutation>, RowOutput)> {
        if self.actor == 0 || self.owner.is_none() || self.lost {
            return None;
        }
        let equipment: Vec<_> = self.equipment.iter().map(Option::as_ref).collect();
        let bag: Vec<_> = self.bag.iter().map(Option::as_ref).collect();
        let bank: Vec<_> = self.bank.iter().map(Option::as_ref).collect();
        let storage = |records, extent, container| match extent {
            Some(extent) => NativeItemTaskStorage::Present {
                records,
                extent,
                container,
            },
            None => NativeItemTaskStorage::Unknown,
        };
        let inventory = NativeItemTaskInventory {
            equipment: storage(&equipment, Some(28), 1),
            bag: storage(&bag, self.bag_extent, 2),
            bank: storage(&bank, self.bank_extent, 3),
        };
        if matches!(task, ItemTask::Move { .. }) {
            let (first, second) = lookup_native_item_task_move(task, &inventory)?;
            let projected = project_native_item_task_move(task, first, second)?;
            Some((
                vec![projected.first, projected.second],
                RowOutput {
                    admitted: projected.admitted,
                    item: None,
                },
            ))
        } else {
            let context = lookup_native_item_task(task, &inventory)?;
            let projected = project_native_item_task(task, context)?;
            Some((
                vec![projected.mutation],
                RowOutput {
                    admitted: projected.admitted,
                    item: projected.output,
                },
            ))
        }
    }
    fn row_mut(&mut self, location: NativeItemTaskLocation) -> Option<&mut Option<Item>> {
        let rows = match location.container {
            1 | 0xfc | 0xf2 => &mut self.equipment,
            2 => &mut self.bag,
            3 => &mut self.bank,
            _ => return None,
        };
        rows.get_mut(usize::from(location.slot))
    }
    fn mutation(&mut self, mutation: NativeItemTaskMutation) {
        match mutation {
            NativeItemTaskMutation::None => {}
            NativeItemTaskMutation::Replace { location, item } => {
                if let Some(row) = self.row_mut(location) {
                    *row = Some(item);
                } else {
                    self.lose();
                }
            }
            NativeItemTaskMutation::Clear { location } => {
                // Actual3924cd30 reset is a known full empty with worldFF.
                if let Some(row) = self.row_mut(location) {
                    *row = Some(empty_equipment());
                } else {
                    self.lose();
                }
            }
        }
    }
    fn forget_id(&mut self, id: u64) {
        for row in self.equipment.iter_mut().chain(&mut self.bag).chain(&mut self.bank) {
            if row.as_ref().is_some_and(|item| item.id == id) {
                *row = None;
            }
        }
    }
    fn forget(&mut self, task: &ItemTask) {
        let mut locations = Vec::new();
        match task {
            ItemTask::Add { container, slot, .. }
            | ItemTask::Count { container, slot, .. }
            | ItemTask::Grade { container, slot, .. }
            | ItemTask::Update { container, slot, .. }
            | ItemTask::Flags { container, slot, .. }
            | ItemTask::Security { container, slot, .. }
            | ItemTask::RemoveSlot { container, slot, .. } => locations.push((*container, *slot)),
            ItemTask::Move {
                from_container,
                from_slot,
                to_container,
                to_slot,
                ..
            } => {
                locations.extend([(*from_container, *from_slot), (*to_container, *to_slot)]);
            }
            ItemTask::Remove { item_id, .. } => self.forget_id(*item_id),
            _ => {}
        }
        // Unprovided item mutations retire the affected physical/ID candidates;
        // they never adopt the legacy reducer's relocated or post-batch result.
        for (container, slot) in locations {
            if let Some(row) = self.row_mut(NativeItemTaskLocation { container, slot }) {
                *row = None;
            }
        }
    }
    pub fn apply_row(&mut self, task: &ItemTask) -> Option<RowOutput> {
        let Some((mutations, output)) = self.project(task) else {
            self.forget(task);
            return None;
        };
        for mutation in mutations {
            self.mutation(mutation);
        }
        Some(output)
    }
    pub fn finish(&mut self, changes: &ItemTasks) {
        // Stronger app certainty retirement for lane mutations: original SC090's
        // serializer reads these IDs, but its handler never dispatches them.
        // In particular this does not emit an admitted item-task observer.
        for id in &changes.force_remove {
            self.forget_id(*id);
        }
    }
    pub fn item(&self, container: u8, slot: u8) -> Option<&Item> {
        if self.actor == 0 || self.lost {
            return None;
        }
        let rows = match container {
            1 if slot < 28 => &self.equipment,
            2 if self.bag_extent.is_some_and(|n| slot < n && slot < 130) => &self.bag,
            3 if self.bank_extent.is_some_and(|n| slot < n && slot < 130) => &self.bank,
            _ => return None,
        };
        rows.get(usize::from(slot))?.as_ref()
    }
    pub fn bag_extent(&self) -> Option<u8> {
        self.bag_extent
    }
    pub fn complete_bag(&self) -> bool {
        self.actor != 0
            && !self.lost
            && self
                .bag_extent
                .is_some_and(|n| n > 0 && n <= 130 && self.bag.iter().take(usize::from(n)).all(Option::is_some))
    }
    pub fn bag_items(&self) -> impl Iterator<Item = (u8, &Item)> {
        let n = if self.complete_bag() { self.bag_extent.unwrap_or(0) } else { 0 };
        self.bag
            .iter()
            .take(usize::from(n))
            .enumerate()
            .filter_map(|(slot, item)| item.as_ref().map(|item| (slot as u8, item)))
    }
}

#[cfg(test)]
mod tests;
