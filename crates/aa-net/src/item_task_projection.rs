//! Native item task outputs, given source-exact pre-row lookup context.
//!
//! Borrowed lookup uses explicit native PRE-row certainty; this never updates `Inventory`,
//! dispatches observers, or retains history.
//! In particular, the legacy inventory reducer's relocation and deletion of nonpositive
//! stacks cannot establish the input to this projection.

use crate::{items::ItemTask, model::Item};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeItemTaskLocation {
    pub container: u8,
    pub slot: u8,
}

/// Context immediately before this individual task, not after the enclosing receipt.
#[derive(Clone, Copy, Debug)]
pub enum NativeItemTaskContext<'a> {
    Unknown,
    /// The actual native lookup returned null. An unhydrated/empty app slot is not proof.
    Missing,
    /// Count: the declared physical slot. Remove: the first native ID lookup result
    /// (equipment 0..27, then bag, then bank). The caller owns lookup completeness.
    Found {
        location: NativeItemTaskLocation,
        item: &'a Item,
    },
}

/// Borrowed, explicitly known native PRE-row storage. No legacy empty-slot inference.
#[derive(Clone, Copy, Debug)]
pub enum NativeItemTaskStorage<'a> {
    Unknown,
    /// Actual native storage pointer is absent, not merely unhydrated.
    Absent,
    Present {
        /// Physical order. None is unknown; Some(empty Item) is an actual record.
        records: &'a [Option<&'a Item>],
        /// Equipment: 28. Containers: explicitly known min(configured, 10*enabled chunks).
        extent: u8,
        /// Native container header byte (equipment uses 1).
        container: u8,
    },
}

#[derive(Clone, Copy, Debug)]
pub struct NativeItemTaskInventory<'a> {
    pub equipment: NativeItemTaskStorage<'a>,
    pub bag: NativeItemTaskStorage<'a>,
    pub bank: NativeItemTaskStorage<'a>,
}

/// Physical Count/Grade/Update lookup or equipment -> bag -> bank Remove lookup.
/// The caller supplies certainty immediately BEFORE this task and owns all mutations.
/// Unknown earlier records prevent a later ID match from becoming authoritative.
/// Equipment indices above 27 are refused as unknown (native physical lookup is unsafe).
pub fn lookup_native_item_task<'a>(
    task: &ItemTask,
    inventory: &NativeItemTaskInventory<'a>,
) -> Option<NativeItemTaskContext<'a>> {
    use NativeItemTaskContext as Context;
    use NativeItemTaskStorage as Storage;
    fn physical<'a>(storage: Storage<'a>, container: u8, slot: u8, equipment: bool) -> Context<'a> {
        match storage {
            Storage::Unknown => Context::Unknown,
            Storage::Absent => Context::Missing,
            Storage::Present {
                records,
                extent,
                container: actual,
            } => {
                if equipment && (extent != 28 || actual != 1 || slot >= 28) {
                    return Context::Unknown;
                }
                if !equipment && (container != actual || slot >= extent) {
                    return Context::Missing;
                }
                if !equipment && slot >= 130 {
                    return Context::Unknown;
                }
                match records.get(usize::from(slot)).copied().flatten() {
                    Some(item) => Context::Found {
                        location: NativeItemTaskLocation { container, slot },
                        item,
                    },
                    None => Context::Unknown,
                }
            }
        }
    }
    fn first_id<'a>(storage: Storage<'a>, id: u64, equipment: bool) -> Context<'a> {
        match storage {
            Storage::Unknown => Context::Unknown,
            Storage::Absent => Context::Missing,
            Storage::Present {
                records,
                extent,
                container,
            } => {
                if equipment && (extent != 28 || container != 1) {
                    return Context::Unknown;
                }
                let safe_extent = if equipment { extent } else { extent.min(130) };
                for slot in 0..safe_extent {
                    let Some(item) = records.get(usize::from(slot)).copied().flatten() else {
                        return Context::Unknown;
                    };
                    // Native equipment checks ID alone; bag/bank skip invalid templates.
                    if (!equipment && item.template_id == 0) || item.id != id {
                        continue;
                    }
                    return Context::Found {
                        location: NativeItemTaskLocation { container, slot },
                        item,
                    };
                }
                if extent > safe_extent {
                    Context::Unknown
                } else {
                    Context::Missing
                }
            }
        }
    }
    match task {
        ItemTask::Count { container: 4, .. }
        | ItemTask::Remove { container: 4, .. }
        | ItemTask::Remove { item_id: 0, .. } => Some(Context::Missing), // no lookup occurs
        ItemTask::Count {
            container, slot, ..
        }
        | ItemTask::Grade {
            container, slot, ..
        }
        | ItemTask::Update {
            container, slot, ..
        } => Some(match container {
            1 | 0xfc | 0xf2 => physical(inventory.equipment, *container, *slot, true),
            2 | 0x64 | 0x65 | 0x66 => physical(inventory.bag, *container, *slot, false),
            3 => physical(inventory.bank, *container, *slot, false),
            _ => Context::Missing,
        }),
        ItemTask::Remove { item_id, .. } => {
            for (storage, equipment) in [
                (inventory.equipment, true),
                (inventory.bag, false),
                (inventory.bank, false),
            ] {
                match first_id(storage, *item_id, equipment) {
                    Context::Missing => {}
                    context => return Some(context),
                }
            }
            Some(Context::Missing)
        }
        ItemTask::Add {
            action: crate::items::action::CREATE,
            container,
            slot,
            ..
        } => {
            if matches!(*container, 4 | 5) {
                return Some(Context::Missing);
            }
            lookup_native_item_task(
                &ItemTask::Count {
                    container: *container,
                    slot: *slot,
                    item_id: 0,
                    delta: 0,
                    template_id: 0,
                },
                inventory,
            )
        }
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum NativeItemTaskMutation {
    None,
    Replace {
        location: NativeItemTaskLocation,
        item: Item,
    },
    Clear {
        location: NativeItemTaskLocation,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativeItemTaskProjection {
    /// Whether native 397d1030 returns true, permitting the later 0x1c notification.
    /// Mutation can occur even when this is false.
    pub admitted: bool,
    /// The operation's output item. None denotes an untouched default output, whose
    /// template is invalid; this does not invent the other default record fields.
    pub output: Option<Item>,
    pub mutation: NativeItemTaskMutation,
}

/// Both physical inputs immediately before a Move8. Packet IDs are not lookup keys.
pub fn lookup_native_item_task_move<'a>(
    task: &ItemTask,
    inventory: &NativeItemTaskInventory<'a>,
) -> Option<(NativeItemTaskContext<'a>, NativeItemTaskContext<'a>)> {
    let ItemTask::Move {
        from_container,
        from_slot,
        to_container,
        to_slot,
        ..
    } = task
    else {
        return None;
    };
    let lookup = |container, slot| {
        if container == 4 {
            return NativeItemTaskContext::Missing;
        }
        lookup_native_item_task(
            &ItemTask::Count {
                container,
                slot,
                item_id: 0,
                delta: 0,
                template_id: 0,
            },
            inventory,
        )
        .unwrap()
    };
    Some((
        lookup(*from_container, *from_slot),
        lookup(*to_container, *to_slot),
    ))
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativeItemTaskMoveProjection {
    pub admitted: bool,
    /// Ordered swap writes. Move has only the dispatcher's cleared invalid output;
    /// neither moved record is an input-consumption output.
    pub first: NativeItemTaskMutation,
    pub second: NativeItemTaskMutation,
}

pub fn project_native_item_task_move(
    task: &ItemTask,
    first: NativeItemTaskContext<'_>,
    second: NativeItemTaskContext<'_>,
) -> Option<NativeItemTaskMoveProjection> {
    let ItemTask::Move {
        from_container,
        from_slot,
        to_container,
        to_slot,
        ..
    } = task
    else {
        return None;
    };
    let untouched = |admitted| NativeItemTaskMoveProjection {
        admitted,
        first: NativeItemTaskMutation::None,
        second: NativeItemTaskMutation::None,
    };
    let resolve = |container, slot, context| match context {
        NativeItemTaskContext::Unknown => None,
        NativeItemTaskContext::Missing => Some(None),
        NativeItemTaskContext::Found { location, item }
            if location == (NativeItemTaskLocation { container, slot }) =>
        {
            Some(Some(item))
        }
        _ => None,
    };
    // Lookup/observer order is first then second. A first missing record returns
    // before the second lookup. Container4 skips only its own lookup.
    let a = if *from_container == 4 {
        None
    } else {
        let Some(item) = resolve(*from_container, *from_slot, first)? else {
            return Some(untouched(false));
        };
        Some(item)
    };
    let b = if *to_container == 4 {
        None
    } else {
        let Some(item) = resolve(*to_container, *to_slot, second)? else {
            return Some(untouched(false));
        };
        Some(item)
    };
    let (Some(a), Some(b)) = (a, b) else {
        return Some(untouched(true));
    };
    let from = NativeItemTaskLocation {
        container: *from_container,
        slot: *from_slot,
    };
    let to = NativeItemTaskLocation {
        container: *to_container,
        slot: *to_slot,
    };
    let same_physical = from == to
        || (matches!(from.container, 1 | 0xfc | 0xf2)
            && matches!(to.container, 1 | 0xfc | 0xf2)
            && from.slot == to.slot);
    if same_physical && a != b {
        return None;
    }
    Some(NativeItemTaskMoveProjection {
        admitted: true,
        first: NativeItemTaskMutation::Replace {
            location: from,
            item: b.clone(),
        },
        second: NativeItemTaskMutation::Replace {
            location: to,
            item: a.clone(),
        },
    })
}

fn qualified_detail(
    detail: &crate::model::ItemDetail,
    payload: Option<&[u8]>,
    raw: Option<&[u8]>,
) -> Option<()> {
    let block = raw?;
    if block.len() != 128 || !matches!(block[0], 0 | 1 | 3) {
        return None;
    }
    let mut reader = crate::wire::Reader::new(block);
    let kind = reader.u8().ok()?;
    let decoded = crate::model::ItemDetail::read(&mut reader, kind).ok()?;
    (decoded == *detail && payload == Some(&block[1..reader.pos()])).then_some(())
}

/// Project original Count4/Remove7/Grade14 and qualified Update9. None means unsupported task, unknown context,
/// or a supplied context inconsistent with the specified native lookup.
///
/// Count ignores the task's ID/template and preserves zero/negative wrapping counts.
/// Remove copies the found item before clearing it, then checks the declared location.
/// Container 4 short-circuits Count/Remove only. Grade copies output before changing
/// the grade; Update copies output after replacing supported normalized detail fields.
/// Update requires a complete 128-byte received block consistent with its parsed fields.
pub fn project_native_item_task(
    task: &ItemTask,
    context: NativeItemTaskContext<'_>,
) -> Option<NativeItemTaskProjection> {
    let (container, slot) = match task {
        ItemTask::Count {
            container, slot, ..
        }
        | ItemTask::Remove {
            container, slot, ..
        }
        | ItemTask::Grade {
            container, slot, ..
        }
        | ItemTask::Update {
            container, slot, ..
        }
        | ItemTask::Add {
            action: crate::items::action::CREATE,
            container,
            slot,
            ..
        } => (*container, *slot),
        _ => return None,
    };
    let untouched = |admitted| NativeItemTaskProjection {
        admitted,
        output: None,
        mutation: NativeItemTaskMutation::None,
    };
    if let ItemTask::Add {
        container,
        item,
        raw_detail_block,
        ..
    } = task
    {
        if matches!(*container, 4 | 5) {
            return Some(untouched(true));
        }
        // Native template0 skips the remaining serialized Item fields; the lane's
        // always-full body cannot establish that constructor-derived record.
        if item.template_id == 0 {
            return None;
        }
        qualified_detail(
            &item.detail,
            item.detail_payload.as_deref(),
            raw_detail_block.as_deref(),
        )?;
    }
    if container == 4 && matches!(task, ItemTask::Count { .. } | ItemTask::Remove { .. }) {
        return Some(untouched(true));
    }
    if let ItemTask::Update {
        detail,
        detail_payload,
        raw_detail_block,
        ..
    } = task
    {
        qualified_detail(
            detail,
            detail_payload.as_deref(),
            raw_detail_block.as_deref(),
        )?;
    }
    if matches!(task, ItemTask::Remove { item_id: 0, .. }) {
        return Some(untouched(false));
    }
    let (location, item) = match context {
        NativeItemTaskContext::Unknown => return None,
        NativeItemTaskContext::Missing => return Some(untouched(false)),
        NativeItemTaskContext::Found { location, item } => (location, item),
    };
    let declared = NativeItemTaskLocation { container, slot };
    match task {
        ItemTask::Add { item: incoming, .. } => {
            if location != declared {
                return None;
            }
            Some(NativeItemTaskProjection {
                admitted: true,
                output: Some(incoming.clone()),
                mutation: NativeItemTaskMutation::Replace {
                    location,
                    item: incoming.clone(),
                },
            })
        }
        ItemTask::Count { delta, .. } => {
            if location != declared {
                return None;
            }
            let mut output = item.clone();
            output.count = output.count.wrapping_add(*delta);
            Some(NativeItemTaskProjection {
                admitted: true,
                mutation: NativeItemTaskMutation::Replace {
                    location,
                    item: output.clone(),
                },
                output: Some(output),
            })
        }
        ItemTask::Remove { item_id, .. } => {
            if item.id != *item_id {
                return None;
            }
            Some(NativeItemTaskProjection {
                admitted: location == declared && item.template_id != 0,
                output: Some(item.clone()),
                mutation: NativeItemTaskMutation::Clear { location },
            })
        }
        ItemTask::Grade { grade, .. } => {
            if location != declared {
                return None;
            }
            let mut changed = item.clone();
            changed.grade = *grade;
            Some(NativeItemTaskProjection {
                admitted: true,
                output: Some(item.clone()), // native copy precedes the grade write
                mutation: NativeItemTaskMutation::Replace {
                    location,
                    item: changed,
                },
            })
        }
        ItemTask::Update {
            detail,
            detail_payload,
            ..
        } => {
            if location != declared {
                return None;
            }
            let mut changed = item.clone();
            changed.detail = detail.clone();
            changed.detail_payload = detail_payload.clone();
            Some(NativeItemTaskProjection {
                admitted: true,
                output: Some(changed.clone()),
                mutation: NativeItemTaskMutation::Replace {
                    location,
                    item: changed,
                },
            })
        }
        _ => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn location(slot: u8) -> NativeItemTaskLocation {
        NativeItemTaskLocation { container: 2, slot }
    }

    fn item(count: i32) -> Item {
        Item {
            id: 123,
            template_id: 8001,
            count,
            ..Default::default()
        }
    }

    fn count(delta: i32) -> ItemTask {
        ItemTask::Count {
            container: 2,
            slot: 3,
            item_id: 999,
            template_id: 9999,
            delta,
        }
    }

    fn remove(slot: u8, id: u64) -> ItemTask {
        ItemTask::Remove {
            container: 2,
            slot,
            item_id: id,
            template_id: 9999,
        }
    }

    #[test]
    fn count_native_oracles_keep_nonpositive_stacks_and_ignore_packet_identity() {
        // Executed original 397cd970: native-enchant-itemtask-output-oracle.json.
        for (before, delta, expected) in
            [(1, -1, 0), (1, -2, -1), (i32::MAX, 1, i32::MIN), (1, 0, 1)]
        {
            let original = item(before);
            let result = project_native_item_task(
                &count(delta),
                NativeItemTaskContext::Found {
                    location: location(3),
                    item: &original,
                },
            )
            .unwrap();
            assert!(result.admitted);
            assert_eq!(
                result
                    .output
                    .as_ref()
                    .map(|i| (i.id, i.template_id, i.count)),
                Some((123, 8001, expected))
            );
            assert!(
                matches!(result.mutation, NativeItemTaskMutation::Replace { item: Item { count, .. }, .. } if count == expected)
            );
            assert_eq!(original.count, before);
        }
    }

    #[test]
    fn remove_native_oracles_copy_before_clear_even_when_admission_fails() {
        let original = item(1);
        for (declared_slot, admitted) in [(3, true), (4, false)] {
            let result = project_native_item_task(
                &remove(declared_slot, 123),
                NativeItemTaskContext::Found {
                    location: location(3),
                    item: &original,
                },
            )
            .unwrap();
            assert_eq!(result.admitted, admitted);
            assert_eq!(result.output, Some(original.clone()));
            assert_eq!(
                result.mutation,
                NativeItemTaskMutation::Clear {
                    location: location(3)
                }
            );
        }
        let result =
            project_native_item_task(&remove(3, 0), NativeItemTaskContext::Unknown).unwrap();
        assert!(!result.admitted);
        assert_eq!(result.mutation, NativeItemTaskMutation::None);
    }

    #[test]
    fn unknown_missing_and_inconsistent_context_are_distinct() {
        assert_eq!(
            project_native_item_task(&count(-1), NativeItemTaskContext::Unknown),
            None
        );
        let missing = project_native_item_task(&count(-1), NativeItemTaskContext::Missing).unwrap();
        assert!(!missing.admitted);
        assert_eq!(missing.output, None);
        let original = item(1);
        assert_eq!(
            project_native_item_task(
                &count(-1),
                NativeItemTaskContext::Found {
                    location: location(4),
                    item: &original
                }
            ),
            None
        );
        assert_eq!(
            project_native_item_task(
                &remove(3, 456),
                NativeItemTaskContext::Found {
                    location: location(3),
                    item: &original
                }
            ),
            None
        );
        assert_eq!(
            project_native_item_task(&ItemTask::Money(1), NativeItemTaskContext::Unknown),
            None
        );
    }

    #[test]
    fn container_four_bypasses_unknown_context_and_zero_id() {
        for task in [
            ItemTask::Count {
                container: 4,
                slot: 255,
                item_id: 0,
                template_id: 0,
                delta: -1,
            },
            ItemTask::Remove {
                container: 4,
                slot: 255,
                item_id: 0,
                template_id: 0,
            },
        ] {
            let result = project_native_item_task(&task, NativeItemTaskContext::Unknown).unwrap();
            assert!(result.admitted);
            assert_eq!(result.output, None);
            assert_eq!(result.mutation, NativeItemTaskMutation::None);
        }
    }

    #[test]
    fn invalid_found_template_still_clears_but_does_not_admit_remove() {
        let original = Item {
            template_id: 0,
            ..item(1)
        };
        let result = project_native_item_task(
            &remove(3, 123),
            NativeItemTaskContext::Found {
                location: location(3),
                item: &original,
            },
        )
        .unwrap();
        assert!(!result.admitted);
        assert_eq!(
            result.mutation,
            NativeItemTaskMutation::Clear {
                location: location(3)
            }
        );
    }

    fn storage<'a>(
        records: &'a [Option<&'a Item>],
        extent: u8,
        container: u8,
    ) -> NativeItemTaskStorage<'a> {
        NativeItemTaskStorage::Present {
            records,
            extent,
            container,
        }
    }

    #[test]
    fn lookup_equipment_precedes_bag_even_for_invalid_template_and_duplicates() {
        let mut empty = item(0);
        empty.id = 0;
        empty.template_id = 0;
        let mut invalid = empty.clone();
        invalid.id = 77;
        let mut valid = item(1);
        valid.id = 77;
        let mut equipment = [Some(&empty); 28];
        equipment[5] = Some(&invalid);
        let bag = [Some(&valid), Some(&valid)];
        let view = NativeItemTaskInventory {
            equipment: storage(&equipment, 28, 1),
            bag: storage(&bag, 2, 2),
            bank: NativeItemTaskStorage::Absent,
        };
        let context = lookup_native_item_task(&remove(0, 77), &view).unwrap();
        assert!(matches!(
            context,
            NativeItemTaskContext::Found {
                location: NativeItemTaskLocation {
                    container: 1,
                    slot: 5
                },
                ..
            }
        ));
        let projected = project_native_item_task(&remove(0, 77), context).unwrap();
        assert!(!projected.admitted);
        assert!(matches!(
            projected.mutation,
            NativeItemTaskMutation::Clear {
                location: NativeItemTaskLocation {
                    container: 1,
                    slot: 5
                }
            }
        ));
    }

    #[test]
    fn lookup_unknown_prefix_blocks_later_matches_and_known_misses_reach_bank() {
        let mut empty = item(0);
        empty.id = 0;
        empty.template_id = 0;
        let mut valid = item(1);
        valid.id = 77;
        let mut equipment = [Some(&empty); 28];
        equipment[3] = None;
        let bag = [Some(&empty), Some(&valid)];
        let bank = [Some(&valid)];
        let mut view = NativeItemTaskInventory {
            equipment: storage(&equipment, 28, 1),
            bag: storage(&bag, 2, 2),
            bank: storage(&bank, 1, 3),
        };
        assert!(matches!(
            lookup_native_item_task(&remove(0, 77), &view),
            Some(NativeItemTaskContext::Unknown)
        ));
        view.equipment = NativeItemTaskStorage::Absent;
        assert!(matches!(
            lookup_native_item_task(&remove(0, 77), &view),
            Some(NativeItemTaskContext::Found {
                location: NativeItemTaskLocation {
                    container: 2,
                    slot: 1
                },
                ..
            })
        ));
        let missed_bag = [Some(&empty), Some(&empty)];
        view.bag = storage(&missed_bag, 2, 2);
        assert!(matches!(
            lookup_native_item_task(&remove(0, 77), &view),
            Some(NativeItemTaskContext::Found {
                location: NativeItemTaskLocation {
                    container: 3,
                    slot: 0
                },
                ..
            })
        ));
        view.bank = NativeItemTaskStorage::Absent;
        assert!(matches!(
            lookup_native_item_task(&remove(0, 77), &view),
            Some(NativeItemTaskContext::Missing)
        ));
    }

    #[test]
    fn physical_count_preserves_empty_record_and_explicit_extent_and_header() {
        let mut empty = item(0);
        empty.id = 0;
        empty.template_id = 0;
        let rows = [Some(&empty), None, Some(&empty), Some(&empty)];
        let mut view = NativeItemTaskInventory {
            equipment: NativeItemTaskStorage::Unknown,
            bag: storage(&rows, 4, 2),
            bank: NativeItemTaskStorage::Absent,
        };
        let context = lookup_native_item_task(&count(3), &view).unwrap();
        let projected = project_native_item_task(&count(3), context).unwrap();
        assert!(projected.admitted);
        assert_eq!(
            (
                projected.output.as_ref().unwrap().template_id,
                projected.output.as_ref().unwrap().count
            ),
            (0, 3)
        );
        view.bag = storage(&rows, 3, 2);
        assert!(matches!(
            lookup_native_item_task(&count(3), &view),
            Some(NativeItemTaskContext::Missing)
        ));
        view.bag = storage(&rows[..2], 4, 2);
        assert!(matches!(
            lookup_native_item_task(&count(3), &view),
            Some(NativeItemTaskContext::Unknown)
        ));
        view.bag = storage(&rows, 4, 0x64);
        assert!(matches!(
            lookup_native_item_task(&count(3), &view),
            Some(NativeItemTaskContext::Missing)
        ));
    }

    #[test]
    fn lookup_bypass_zero_remove_and_unsafe_equipment_are_explicit() {
        let mut view = NativeItemTaskInventory {
            equipment: NativeItemTaskStorage::Unknown,
            bag: NativeItemTaskStorage::Unknown,
            bank: NativeItemTaskStorage::Unknown,
        };
        assert!(matches!(
            lookup_native_item_task(&remove(0, 0), &view),
            Some(NativeItemTaskContext::Missing)
        ));
        let bypass = ItemTask::Count {
            container: 4,
            slot: 200,
            item_id: 1,
            template_id: 1,
            delta: 1,
        };
        let context = lookup_native_item_task(&bypass, &view).unwrap();
        assert!(project_native_item_task(&bypass, context).unwrap().admitted);
        let mut empty = item(0);
        empty.id = 0;
        empty.template_id = 0;
        let rows = [Some(&empty); 28];
        view.equipment = storage(&rows, 28, 1);
        let unsafe_count = ItemTask::Count {
            container: 1,
            slot: 28,
            item_id: 1,
            template_id: 1,
            delta: 1,
        };
        assert!(matches!(
            lookup_native_item_task(&unsafe_count, &view),
            Some(NativeItemTaskContext::Unknown)
        ));
    }
    #[test]
    fn oversized_native_header_preserves_safe_hit_but_cannot_prove_miss_or_out_of_storage() {
        let mut empty = item(0);
        empty.id = 0;
        empty.template_id = 0;
        let mut valid = item(1);
        valid.id = 77;
        let mut rows = [Some(&empty); 200];
        rows[2] = Some(&valid);
        let view = NativeItemTaskInventory {
            equipment: NativeItemTaskStorage::Absent,
            bag: NativeItemTaskStorage::Absent,
            bank: storage(&rows, 200, 3),
        };
        assert!(matches!(
            lookup_native_item_task(&remove(0, 77), &view),
            Some(NativeItemTaskContext::Found {
                location: NativeItemTaskLocation {
                    container: 3,
                    slot: 2
                },
                ..
            })
        ));
        assert!(matches!(
            lookup_native_item_task(&remove(0, 999), &view),
            Some(NativeItemTaskContext::Unknown)
        ));
        let count = ItemTask::Count {
            container: 3,
            slot: 130,
            item_id: 1,
            template_id: 1,
            delta: 1,
        };
        assert!(matches!(
            lookup_native_item_task(&count, &view),
            Some(NativeItemTaskContext::Unknown)
        ));
    }

    fn update_task() -> ItemTask {
        // Independent literal Update9 body, Equip detail with image42/durability13.
        let mut wire = vec![40, 1, 9, 2, 3, 0xe7, 3, 0, 0, 0, 0, 0, 0, 128, 0];
        let mut block = vec![0xa5; 128];
        block[..56].fill(0);
        block[0] = 1;
        block[1] = 42;
        block[5] = 13;
        wire.extend_from_slice(&block);
        wire.push(0); // no force removals
        let tasks = crate::items::ItemTasks::read(&mut crate::wire::Reader::new(&wire)).unwrap();
        assert!(!tasks.truncated);
        tasks.tasks.into_iter().next().unwrap()
    }

    #[test]
    fn grade_native_oracle_copies_old_output_before_new_grade_and_ignores_packet_id() {
        // Executed397cde80: native-item-task-grade-update-oracle.json.
        let mut original = item(5);
        original.grade = 6;
        let task = ItemTask::Grade {
            container: 2,
            slot: 3,
            item_id: 999,
            grade: 8,
        };
        let rows = [None, None, None, Some(&original)];
        let view = NativeItemTaskInventory {
            equipment: NativeItemTaskStorage::Unknown,
            bag: storage(&rows, 4, 2),
            bank: NativeItemTaskStorage::Unknown,
        };
        let context = lookup_native_item_task(&task, &view).unwrap();
        let projected = project_native_item_task(&task, context).unwrap();
        assert!(projected.admitted);
        assert_eq!(projected.output, Some(original.clone()));
        let NativeItemTaskMutation::Replace {
            location: at,
            item: changed,
        } = projected.mutation
        else {
            panic!()
        };
        assert_eq!(at, location(3));
        let mut expected = original;
        expected.grade = 8;
        assert_eq!(changed, expected);
        assert!(
            !project_native_item_task(&task, NativeItemTaskContext::Missing)
                .unwrap()
                .admitted
        );
    }

    #[test]
    fn update_native_oracle_changes_only_detail_and_copies_new_output() {
        // Executed397cdc10: native-item-task-detail-update-oracle.json.
        let original = item(5);
        let task = update_task();
        let projected = project_native_item_task(
            &task,
            NativeItemTaskContext::Found {
                location: location(3),
                item: &original,
            },
        )
        .unwrap();
        assert!(projected.admitted);
        let NativeItemTaskMutation::Replace { item: changed, .. } = projected.mutation else {
            panic!()
        };
        assert_eq!(projected.output, Some(changed.clone()));
        let mut expected = original.clone();
        if let ItemTask::Update {
            detail,
            detail_payload,
            ..
        } = task
        {
            expected.detail = detail;
            expected.detail_payload = detail_payload;
        }
        assert_eq!(changed, expected);
        assert_eq!(
            (changed.id, changed.template_id, changed.count),
            (123, 8001, 5)
        );
        assert_eq!(original.detail, crate::model::ItemDetail::None);
    }

    #[test]
    fn update_refuses_missing_unknown_malformed_or_inconsistent_native_inputs() {
        let original = item(5);
        let task = update_task();
        let context = NativeItemTaskContext::Found {
            location: location(3),
            item: &original,
        };
        let mut synthetic = task.clone();
        if let ItemTask::Update {
            raw_detail_block, ..
        } = &mut synthetic
        {
            *raw_detail_block = None;
        }
        assert!(project_native_item_task(&synthetic, context).is_none());
        for len in [0, 4, 127, 129] {
            let mut malformed = task.clone();
            if let ItemTask::Update {
                raw_detail_block, ..
            } = &mut malformed
            {
                *raw_detail_block = Some(vec![1; len]);
            }
            assert!(project_native_item_task(&malformed, context).is_none());
        }
        for which in 0..3 {
            let mut mismatch = task.clone();
            if let ItemTask::Update {
                detail,
                detail_payload,
                raw_detail_block,
                ..
            } = &mut mismatch
            {
                match which {
                    0 => raw_detail_block.as_mut().unwrap()[0] = 0xfe,
                    1 => *detail = crate::model::ItemDetail::None,
                    _ => *detail_payload = None,
                }
            }
            assert!(project_native_item_task(&mismatch, context).is_none());
        }
        assert!(project_native_item_task(&task, NativeItemTaskContext::Unknown).is_none());
        assert!(
            project_native_item_task(
                &task,
                NativeItemTaskContext::Found {
                    location: location(2),
                    item: &original
                }
            )
            .is_none()
        );
        let missing = project_native_item_task(&task, NativeItemTaskContext::Missing).unwrap();
        assert!(!missing.admitted);
        assert_eq!(missing.mutation, NativeItemTaskMutation::None);
        assert_eq!(missing.output, None);
    }
    #[test]
    fn normalized_none_and_mate_details_require_exact_received_prefixes() {
        let original = item(5);
        for (tag, detail, payload) in [
            (0, crate::model::ItemDetail::None, vec![]),
            (
                3,
                crate::model::ItemDetail::Mate {
                    exp: -1,
                    dead: 0xfe,
                    level: 0xff,
                },
                vec![0xff, 0xff, 0xff, 0xff, 0xfe, 0xff],
            ),
        ] {
            let mut block = vec![0xa5; 128];
            block[0] = tag;
            block[1..1 + payload.len()].copy_from_slice(&payload);
            let task = ItemTask::Update {
                container: 2,
                slot: 3,
                item_id: 999,
                detail: detail.clone(),
                detail_payload: Some(payload.clone()),
                raw_detail_block: Some(block),
            };
            let result = project_native_item_task(
                &task,
                NativeItemTaskContext::Found {
                    location: location(3),
                    item: &original,
                },
            )
            .unwrap();
            assert!(result.admitted);
            assert_eq!(result.output.as_ref().unwrap().detail, detail);
            assert_eq!(
                result.output.as_ref().unwrap().detail_payload,
                Some(payload)
            );
        }
        let task = ItemTask::Grade {
            container: 4,
            slot: 0,
            item_id: 999,
            grade: 8,
        };
        let unknown = NativeItemTaskStorage::Unknown;
        let inventory = NativeItemTaskInventory {
            equipment: unknown,
            bag: unknown,
            bank: unknown,
        };
        let context = lookup_native_item_task(&task, &inventory).unwrap();
        assert!(!project_native_item_task(&task, context).unwrap().admitted); // no Count/Remove bypass
    }
    #[test]
    fn add_native_oracle_replaces_full_record_and_outputs_after_assignment() {
        let original = item(5);
        let mut incoming = item(17);
        incoming.id = 999;
        incoming.template_id = 9001;
        incoming.grade = 8;
        incoming.flags = 4;
        incoming.world_id = 7;
        incoming.made_unit_id = 55;
        incoming.unsecure_time = 66;
        incoming.unpack_time = 77;
        incoming.detail_payload = Some(vec![]);
        let make = |raw| ItemTask::Add {
            action: crate::items::action::CREATE,
            container: 2,
            slot: 3,
            item: incoming.clone(),
            raw_detail_block: raw,
        };
        let context = NativeItemTaskContext::Found {
            location: location(3),
            item: &original,
        };
        let raw = vec![0; 128];
        let result = project_native_item_task(&make(Some(raw.clone())), context).unwrap();
        assert!(result.admitted);
        assert_eq!(result.output, Some(incoming.clone()));
        assert_eq!(
            result.mutation,
            NativeItemTaskMutation::Replace {
                location: location(3),
                item: incoming.clone()
            }
        );
        assert!(
            !project_native_item_task(&make(Some(raw.clone())), NativeItemTaskContext::Missing)
                .unwrap()
                .admitted
        );
        assert!(
            project_native_item_task(&make(Some(raw.clone())), NativeItemTaskContext::Unknown)
                .is_none()
        );
        for block in [
            None,
            Some(vec![0; 127]),
            Some(vec![0; 129]),
            Some(vec![0xfe; 128]),
        ] {
            assert!(project_native_item_task(&make(block), context).is_none());
        }
        for container in [4, 5] {
            let mut task = make(None);
            if let ItemTask::Add { container: c, .. } = &mut task {
                *c = container;
            }
            let result = project_native_item_task(&task, NativeItemTaskContext::Unknown).unwrap();
            assert!(result.admitted);
            assert_eq!(result.output, None);
            assert_eq!(result.mutation, NativeItemTaskMutation::None);
        }
        let mut task = make(Some(raw));
        if let ItemTask::Add { action, .. } = &mut task {
            *action = 15;
        }
        assert!(project_native_item_task(&task, context).is_none());
    }

    fn move_task(from: u8, to: u8) -> ItemTask {
        ItemTask::Move {
            from_container: from,
            from_slot: 3,
            from_item_id: 999,
            to_container: to,
            to_slot: 4,
            to_item_id: 888,
        }
    }

    #[test]
    fn move_native_oracles_swap_physical_records_ignore_ids_and_preserve_pre_rows() {
        let a = item(5);
        let mut b = item(9);
        b.id = 456;
        b.template_id = 8002;
        let first = NativeItemTaskContext::Found {
            location: location(3),
            item: &a,
        };
        let second = NativeItemTaskContext::Found {
            location: location(4),
            item: &b,
        };
        let result = project_native_item_task_move(&move_task(2, 2), first, second).unwrap();
        assert!(result.admitted);
        assert_eq!(
            result.first,
            NativeItemTaskMutation::Replace {
                location: location(3),
                item: b.clone()
            }
        );
        assert_eq!(
            result.second,
            NativeItemTaskMutation::Replace {
                location: location(4),
                item: a.clone()
            }
        );
        assert_eq!(a.id, 123);
        assert_eq!(b.id, 456);
        for (one, two, expected) in [
            (first, NativeItemTaskContext::Missing, false),
            (
                NativeItemTaskContext::Missing,
                NativeItemTaskContext::Unknown,
                false,
            ),
        ] {
            let result = project_native_item_task_move(&move_task(2, 2), one, two).unwrap();
            assert_eq!(result.admitted, expected);
            assert_eq!(result.first, NativeItemTaskMutation::None);
            assert_eq!(result.second, NativeItemTaskMutation::None);
        }
        assert!(
            project_native_item_task_move(&move_task(2, 2), first, NativeItemTaskContext::Unknown)
                .is_none()
        );
        for (task, one, two) in [
            (move_task(4, 2), NativeItemTaskContext::Unknown, second),
            (move_task(2, 4), first, NativeItemTaskContext::Unknown),
            (
                move_task(4, 4),
                NativeItemTaskContext::Unknown,
                NativeItemTaskContext::Unknown,
            ),
        ] {
            let result = project_native_item_task_move(&task, one, two).unwrap();
            assert!(result.admitted);
            assert_eq!(result.first, NativeItemTaskMutation::None);
            assert_eq!(result.second, NativeItemTaskMutation::None);
        }
        assert!(
            !project_native_item_task_move(
                &move_task(4, 2),
                NativeItemTaskContext::Unknown,
                NativeItemTaskContext::Missing
            )
            .unwrap()
            .admitted
        );
    }

    #[test]
    fn move_same_slot_requires_consistent_pre_record_and_lookup_preserves_unknowns() {
        let a = item(5);
        let b = item(6);
        let mut task = move_task(2, 2);
        if let ItemTask::Move { to_slot, .. } = &mut task {
            *to_slot = 3;
        }
        let first = NativeItemTaskContext::Found {
            location: location(3),
            item: &a,
        };
        assert!(
            project_native_item_task_move(&task, first, first)
                .unwrap()
                .admitted
        );
        assert!(
            project_native_item_task_move(
                &task,
                first,
                NativeItemTaskContext::Found {
                    location: location(3),
                    item: &b
                }
            )
            .is_none()
        );
        let rows = [None, None, None, Some(&a), Some(&b)];
        let inventory = NativeItemTaskInventory {
            equipment: NativeItemTaskStorage::Absent,
            bag: NativeItemTaskStorage::Present {
                records: &rows,
                extent: 5,
                container: 2,
            },
            bank: NativeItemTaskStorage::Unknown,
        };
        let (one, two) = lookup_native_item_task_move(&move_task(2, 2), &inventory).unwrap();
        assert!(
            project_native_item_task_move(&move_task(2, 2), one, two)
                .unwrap()
                .admitted
        );
        let mut unknown = move_task(2, 2);
        if let ItemTask::Move { to_slot, .. } = &mut unknown {
            *to_slot = 2;
        }
        let (one, two) = lookup_native_item_task_move(&unknown, &inventory).unwrap();
        assert!(project_native_item_task_move(&unknown, one, two).is_none());
    }
}
