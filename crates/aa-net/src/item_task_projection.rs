//! Native Count/Remove task outputs, given source-exact pre-row lookup context.
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

/// Source-exact physical Count lookup or equipment -> bag -> bank Remove lookup.
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

/// Project original 397cd970/397cdad0. None means unsupported task, unknown context,
/// or a supplied context inconsistent with the specified native lookup.
///
/// Count ignores the task's ID/template and preserves zero/negative wrapping counts.
/// Remove copies the found item before clearing it, then checks the declared location.
/// Container 4 short-circuits both helpers before any lookup or mutation.
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
        } => (*container, *slot),
        _ => return None,
    };
    let untouched = |admitted| NativeItemTaskProjection {
        admitted,
        output: None,
        mutation: NativeItemTaskMutation::None,
    };
    if container == 4 {
        return Some(untouched(true));
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
}
