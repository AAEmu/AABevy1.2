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

/// Physical task lookup or equipment -> bag -> bank Remove lookup. Owner15 uses
/// container storage only; original no-lookup branches return Missing.
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
        }
        | ItemTask::Flags {
            container, slot, ..
        }
        | ItemTask::Security {
            container, slot, ..
        }
        | ItemTask::RemoveSlot {
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
            action:
                crate::items::action::CREATE
                | crate::items::action::TAKE
                | crate::items::action::CHANGE_OWNER,
            container,
            slot,
            ..
        } => {
            let ItemTask::Add { action, .. } = task else {
                unreachable!()
            };
            if *action == crate::items::action::CHANGE_OWNER
                && !matches!(*container, 2 | 0x64 | 0x65 | 0x66 | 3)
            {
                return Some(Context::Missing);
            }
            if *action != crate::items::action::CHANGE_OWNER && matches!(*container, 4 | 5) {
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
        ItemTask::RemoveCrafting { .. } => Some(Context::Missing),
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

/// Original SC093 uses ONLY the static BAG container, never equipment or bank.
pub fn lookup_native_item_detail_update<'a>(
    update: &crate::items::ItemDetailUpdate,
    inventory: &NativeItemTaskInventory<'a>,
) -> NativeItemTaskContext<'a> {
    if update.container != 2 {
        return NativeItemTaskContext::Missing;
    }
    lookup_native_item_task(
        &ItemTask::Count {
            container: 2,
            slot: update.slot,
            item_id: 0,
            delta: 0,
            template_id: 0,
        },
        inventory,
    )
    .unwrap()
}

/// No task output or0x1c admission. A Replace denotes actual detail mutation,
/// after which native394e47b0 releases this packed BAG key with pending=false.
/// None is unknown/malformed context; Some(None) is a proven native no-op.
pub fn project_native_item_detail_update(
    update: &crate::items::ItemDetailUpdate,
    context: NativeItemTaskContext<'_>,
) -> Option<NativeItemTaskMutation> {
    if update.container != 2 {
        return Some(NativeItemTaskMutation::None);
    }
    let detail = update.detail()?;
    match context {
        NativeItemTaskContext::Unknown => None,
        NativeItemTaskContext::Missing => Some(NativeItemTaskMutation::None),
        NativeItemTaskContext::Found { location, item } => {
            if location
                != (NativeItemTaskLocation {
                    container: 2,
                    slot: update.slot,
                })
            {
                return None;
            }
            if item.id != update.item_id || item.template_id == 0 {
                return Some(NativeItemTaskMutation::None);
            }
            let mut changed = item.clone();
            changed.detail = detail;
            changed.detail_payload = Some(update.raw_detail[1..].to_vec());
            Some(NativeItemTaskMutation::Replace {
                location,
                item: changed,
            })
        }
    }
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
    // Original3961edf0 tag8 is exactly eight opaque MusicSheet bytes. Keep the
    // same complete received union + reparsed prefix checks as tags0/1/3.
    if block.len() != 128 || !matches!(block[0], 0 | 1 | 3 | 8) {
        return None;
    }
    let mut reader = crate::wire::Reader::new(block);
    let kind = reader.u8().ok()?;
    let decoded = crate::model::ItemDetail::read(&mut reader, kind).ok()?;
    (decoded == *detail && payload == Some(&block[1..reader.pos()])).then_some(())
}

/// Project original supported single-row Item tasks. None means unsupported task, unknown context,
/// or a supplied context inconsistent with the specified native lookup.
///
/// Count ignores the task's ID/template and preserves zero/negative wrapping counts.
/// Remove copies the found item before clearing it, then checks the declared location.
/// Container 4 short-circuits Count/Remove; Add5/Take6 also bypass container5.
/// Owner15 never bypasses. Grade copies output before changing
/// the grade; Update copies output after replacing supported normalized detail fields.
/// Update requires a complete 128-byte received block consistent with its parsed fields.
/// Security11 nonzero unpack of equipment remains unsupported without its static provider.
pub fn project_native_item_task(
    task: &ItemTask,
    context: NativeItemTaskContext<'_>,
) -> Option<NativeItemTaskProjection> {
    project_native_item_task_with_recharge(task, context, None)
}

/// Optional source-complete static result for Security11 unpack. Some(0) means
/// known no recharge (including genuinely missing static descriptor/class row),
/// Some(nonzero) is the resolved recharge buff ID; None remains unknown.
/// It must describe the actual PRE-row item's template, not the packet ID.
pub fn project_native_item_task_with_recharge(
    task: &ItemTask,
    context: NativeItemTaskContext<'_>,
    known_recharge_buff: Option<u32>,
) -> Option<NativeItemTaskProjection> {
    if matches!(task, ItemTask::RemoveCrafting { .. }) {
        return Some(NativeItemTaskProjection {
            admitted: false,
            output: None,
            mutation: NativeItemTaskMutation::None,
        });
    }
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
        | ItemTask::Flags {
            container, slot, ..
        }
        | ItemTask::Security {
            container, slot, ..
        }
        | ItemTask::RemoveSlot {
            container, slot, ..
        }
        | ItemTask::Add {
            action:
                crate::items::action::CREATE
                | crate::items::action::TAKE
                | crate::items::action::CHANGE_OWNER,
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
        action,
        container,
        item,
        raw_detail_block,
        ..
    } = task
    {
        if *action == crate::items::action::CHANGE_OWNER
            && !matches!(*container, 2 | 0x64 | 0x65 | 0x66 | 3)
        {
            return Some(untouched(false));
        }
        if *action != crate::items::action::CHANGE_OWNER && matches!(*container, 4 | 5) {
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
        ItemTask::Flags { bits, .. } | ItemTask::Security { bits, .. } => {
            if location != declared {
                return None;
            }
            let mut changed = item.clone();
            if let ItemTask::Security {
                unsecure_time,
                unpack_time,
                is_unpack,
                ..
            } = task
            {
                changed.flags = *bits;
                changed.unsecure_time = *unsecure_time;
                changed.unpack_time = *unpack_time;
                if *is_unpack != 0 && changed.detail.kind() == 1 {
                    if known_recharge_buff? != 0 {
                        let crate::model::ItemDetail::Equip(equip) = &mut changed.detail else {
                            return None;
                        };
                        let mut payload = changed.detail_payload.clone()?;
                        let mut reader = crate::wire::Reader::new(&payload);
                        let decoded = crate::model::ItemDetail::read(&mut reader, 1).ok()?;
                        if decoded != item.detail || !reader.is_empty() {
                            return None;
                        }
                        payload
                            .get_mut(11..19)?
                            .copy_from_slice(&unpack_time.to_le_bytes());
                        equip.charge_time = *unpack_time;
                        changed.detail_payload = Some(payload);
                    }
                }
            } else {
                changed.flags |= *bits;
            }
            Some(NativeItemTaskProjection {
                admitted: true,
                output: Some(changed.clone()),
                mutation: NativeItemTaskMutation::Replace {
                    location,
                    item: changed,
                },
            })
        }
        ItemTask::RemoveSlot { .. } => {
            if location != declared {
                return None;
            }
            Some(NativeItemTaskProjection {
                admitted: true,
                output: Some(item.clone()),
                mutation: NativeItemTaskMutation::Clear { location },
            })
        }
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
            *action = 99;
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
    #[test]
    fn physical_flags_security_and_seize_match_original_helper_vectors() {
        let mut original = item(5);
        original.flags = 0x81;
        original.unsecure_time = 66;
        original.unpack_time = 77;
        let context = NativeItemTaskContext::Found {
            location: location(3),
            item: &original,
        };
        let flags = ItemTask::Flags {
            container: 2,
            slot: 3,
            item_id: 999,
            bits: 4,
        };
        let security = ItemTask::Security {
            container: 2,
            slot: 3,
            item_id: 999,
            bits: 4,
            is_unsecure_excess: 255,
            is_unsecure_set: 255,
            is_unpack: 0,
            unsecure_time: -7,
            unpack_time: -9,
        };
        for (task, expected_flags, unsecure, unpack) in
            [(&flags, 0x85, 66, 77), (&security, 4, -7, -9)]
        {
            let result = project_native_item_task(task, context).unwrap();
            assert!(result.admitted);
            let mut expected = original.clone();
            expected.flags = expected_flags;
            expected.unsecure_time = unsecure;
            expected.unpack_time = unpack;
            assert_eq!(result.output, Some(expected.clone()));
            assert_eq!(
                result.mutation,
                NativeItemTaskMutation::Replace {
                    location: location(3),
                    item: expected
                }
            );
            assert!(
                !project_native_item_task(task, NativeItemTaskContext::Missing)
                    .unwrap()
                    .admitted
            );
            assert!(project_native_item_task(task, NativeItemTaskContext::Unknown).is_none());
        }
        let seize = ItemTask::RemoveSlot {
            container: 2,
            slot: 3,
            item_id: 999,
        };
        let result = project_native_item_task(&seize, context).unwrap();
        assert!(result.admitted);
        assert_eq!(result.output, Some(original.clone()));
        assert_eq!(
            result.mutation,
            NativeItemTaskMutation::Clear {
                location: location(3)
            }
        );
        let empty = Item::default();
        let result = project_native_item_task(
            &seize,
            NativeItemTaskContext::Found {
                location: location(3),
                item: &empty,
            },
        )
        .unwrap();
        assert!(result.admitted);
        assert_eq!(result.output, Some(empty));
        for value in [1, 2, 255] {
            let mut task = security.clone();
            if let ItemTask::Security { is_unpack, .. } = &mut task {
                *is_unpack = value;
            }
            assert_eq!(project_native_item_task(&task, context), project_native_item_task(&security, context));
        }
        let mut wrong = flags.clone();
        if let ItemTask::Flags { slot, .. } = &mut wrong {
            *slot = 4;
        }
        assert!(project_native_item_task(&wrong, context).is_none());
        assert_eq!(original.flags, 0x81);
    }

    #[test]
    fn take_and_owner_share_qualified_copy_but_have_distinct_container_admission() {
        let original = item(5);
        let mut incoming = item(17);
        incoming.id = 999;
        incoming.detail_payload = Some(vec![]);
        for action in [
            crate::items::action::TAKE,
            crate::items::action::CHANGE_OWNER,
        ] {
            let make = |container, raw| ItemTask::Add {
                action,
                container,
                slot: 3,
                item: incoming.clone(),
                raw_detail_block: raw,
            };
            let result = project_native_item_task(
                &make(2, Some(vec![0; 128])),
                NativeItemTaskContext::Found {
                    location: location(3),
                    item: &original,
                },
            )
            .unwrap();
            assert!(result.admitted);
            assert_eq!(result.output, Some(incoming.clone()));
            assert!(
                project_native_item_task(
                    &make(2, None),
                    NativeItemTaskContext::Found {
                        location: location(3),
                        item: &original
                    }
                )
                .is_none()
            );
            for container in [4, 5] {
                assert_eq!(
                    project_native_item_task(
                        &make(container, None),
                        NativeItemTaskContext::Unknown
                    )
                    .unwrap()
                    .admitted,
                    action == crate::items::action::TAKE
                );
            }
            if action == crate::items::action::CHANGE_OWNER {
                for container in [1, 0xfc, 0xf2] {
                    assert!(
                        !project_native_item_task(
                            &make(container, None),
                            NativeItemTaskContext::Unknown
                        )
                        .unwrap()
                        .admitted
                    );
                }
                let bank_loc = NativeItemTaskLocation {
                    container: 3,
                    slot: 3,
                };
                assert!(
                    project_native_item_task(
                        &make(3, Some(vec![0; 128])),
                        NativeItemTaskContext::Found {
                            location: bank_loc,
                            item: &original
                        }
                    )
                    .unwrap()
                    .admitted
                );
            }
        }
    }

    #[test]
    fn native_remove_crafting_is_noop_and_physical_lookup_ignores_packet_identity() {
        let original = item(5);
        let rows = [None, None, None, Some(&original)];
        let inventory = NativeItemTaskInventory {
            equipment: NativeItemTaskStorage::Unknown,
            bag: NativeItemTaskStorage::Present {
                records: &rows,
                extent: 4,
                container: 2,
            },
            bank: NativeItemTaskStorage::Unknown,
        };
        for task in [
            ItemTask::Flags {
                container: 2,
                slot: 3,
                item_id: 999,
                bits: 4,
            },
            ItemTask::RemoveSlot {
                container: 2,
                slot: 3,
                item_id: 0,
            },
        ] {
            let context = lookup_native_item_task(&task, &inventory).unwrap();
            assert!(project_native_item_task(&task, context).unwrap().admitted);
        }
        let task = ItemTask::RemoveCrafting { item_id: 123 };
        let result = project_native_item_task(&task, NativeItemTaskContext::Unknown).unwrap();
        assert!(!result.admitted);
        assert_eq!(result.output, None);
        assert_eq!(result.mutation, NativeItemTaskMutation::None);
    }
    #[test]
    fn unpack_recharge_matches_original_static_branch_and_preserves_reserved_payload() {
        use crate::model::{EquipDetail, ItemDetail};
        let mut original = item(5);
        let mut payload = vec![0; 55];
        payload[5] = 0xa5;
        payload[6] = 0xf2;
        payload[11..19].copy_from_slice(&102i64.to_le_bytes());
        original.detail = ItemDetail::Equip(EquipDetail {
            charge_time: 102,
            ..Default::default()
        });
        original.detail_payload = Some(payload.clone());
        let context = NativeItemTaskContext::Found {
            location: location(3),
            item: &original,
        };
        let task = ItemTask::Security {
            container: 2,
            slot: 3,
            item_id: 999,
            bits: 4,
            is_unsecure_excess: 0,
            is_unsecure_set: 0,
            is_unpack: 255,
            unsecure_time: -7,
            unpack_time: -9,
        };
        assert!(project_native_item_task(&task, context).is_none());
        for recharge in [0, 42, u32::MAX] {
            let result =
                project_native_item_task_with_recharge(&task, context, Some(recharge)).unwrap();
            assert!(result.admitted);
            let expected_charge = if recharge == 0 { 102 } else { -9 };
            let mut expected = original.clone();
            expected.flags = 4;
            expected.unsecure_time = -7;
            expected.unpack_time = -9;
            if let ItemDetail::Equip(detail) = &mut expected.detail {
                detail.charge_time = expected_charge;
            }
            expected.detail_payload.as_mut().unwrap()[11..19]
                .copy_from_slice(&expected_charge.to_le_bytes());
            assert_eq!(result.output, Some(expected.clone()));
            assert_eq!(
                result.mutation,
                NativeItemTaskMutation::Replace {
                    location: location(3),
                    item: expected
                }
            );
        }
        assert_eq!(original.detail_payload, Some(payload));
        assert!(
            !project_native_item_task_with_recharge(
                &task,
                NativeItemTaskContext::Missing,
                Some(42)
            )
            .unwrap()
            .admitted
        );
        let mut no_detail = item(5);
        no_detail.detail = ItemDetail::None;
        assert!(
            project_native_item_task_with_recharge(
                &task,
                NativeItemTaskContext::Found {
                    location: location(3),
                    item: &no_detail
                },
                Some(42)
            )
            .unwrap()
            .admitted
        );
    }

    #[test]
    fn unpack_non_equipment_and_missing_pre_row_skip_static_recharge_provider() {
        use crate::model::ItemDetail;
        let task = ItemTask::Security {
            container: 2,
            slot: 3,
            item_id: 999,
            bits: 4,
            is_unsecure_excess: 0,
            is_unsecure_set: 0,
            is_unpack: 255,
            unsecure_time: -7,
            unpack_time: -9,
        };
        for detail in [
            ItemDetail::None,
            ItemDetail::Other(3),
            ItemDetail::Other(4),
            ItemDetail::Other(255),
        ] {
            let mut original = item(5);
            original.detail = detail;
            let context = NativeItemTaskContext::Found {
                location: location(3),
                item: &original,
            };
            let output = project_native_item_task(&task, context).unwrap();
            let mut expected = original.clone();
            expected.flags = 4;
            expected.unsecure_time = -7;
            expected.unpack_time = -9;
            assert!(output.admitted);
            assert_eq!(output.output, Some(expected.clone()));
            assert_eq!(
                output.mutation,
                NativeItemTaskMutation::Replace {
                    location: location(3),
                    item: expected
                }
            );
        }
        assert_eq!(
            project_native_item_task(&task, NativeItemTaskContext::Missing),
            Some(NativeItemTaskProjection {
                admitted: false,
                output: None,
                mutation: NativeItemTaskMutation::None
            })
        );
        assert!(project_native_item_task(&task, NativeItemTaskContext::Unknown).is_none());
    }

    #[test]
    fn unpack_recharge_refuses_unknown_or_inconsistent_equipment_payload_only_when_needed() {
        use crate::model::{EquipDetail, ItemDetail};
        let mut original = item(5);
        original.detail = ItemDetail::Equip(EquipDetail::default());
        let task = ItemTask::Security {
            container: 2,
            slot: 3,
            item_id: 999,
            bits: 4,
            is_unsecure_excess: 0,
            is_unsecure_set: 0,
            is_unpack: 1,
            unsecure_time: i64::MIN,
            unpack_time: i64::MAX,
        };
        for bad in [
            None,
            Some(vec![]),
            Some(vec![0; 54]),
            Some(vec![0; 56]),
            Some(vec![0xff; 55]),
        ] {
            original.detail_payload = bad;
            let context = NativeItemTaskContext::Found {
                location: location(3),
                item: &original,
            };
            assert!(project_native_item_task_with_recharge(&task, context, None).is_none());
            assert!(project_native_item_task_with_recharge(&task, context, Some(42)).is_none());
            let known_absent =
                project_native_item_task_with_recharge(&task, context, Some(0)).unwrap();
            assert!(known_absent.admitted);
            assert_eq!(
                known_absent.output.unwrap().detail_payload,
                original.detail_payload
            );
        }
        original.detail = ItemDetail::Other(1);
        original.detail_payload = Some(vec![0; 55]);
        assert!(
            project_native_item_task_with_recharge(
                &task,
                NativeItemTaskContext::Found {
                    location: location(3),
                    item: &original
                },
                Some(42)
            )
            .is_none()
        );
        original.detail = ItemDetail::Equip(EquipDetail::default());
        original.detail_payload = Some(vec![0; 55]);
        let result = project_native_item_task_with_recharge(
            &task,
            NativeItemTaskContext::Found {
                location: location(3),
                item: &original,
            },
            Some(42),
        )
        .unwrap();
        let changed = result.output.unwrap();
        assert_eq!(changed.unsecure_time, i64::MIN);
        assert_eq!(changed.unpack_time, i64::MAX);
        assert_eq!(
            &changed.detail_payload.unwrap()[11..19],
            &i64::MAX.to_le_bytes()
        );
    }
    #[test]
    fn detail_update_native_mutation_gates_and_full_metadata_preservation() {
        let mut original = item(5);
        original.flags = 0x81;
        original.world_id = 7;
        original.unsecure_time = -7;
        original.unpack_time = -9;
        let update = crate::items::ItemDetailUpdate {
            item_id: 123,
            container: 2,
            slot: 3,
            raw_detail: vec![3, 0xff, 0xff, 0xff, 0xff, 0xf2, 0xfe],
        };
        let context = NativeItemTaskContext::Found {
            location: location(3),
            item: &original,
        };
        let NativeItemTaskMutation::Replace {
            location: at,
            item: changed,
        } = project_native_item_detail_update(&update, context).unwrap()
        else {
            panic!()
        };
        let mut expected = original.clone();
        expected.detail = crate::model::ItemDetail::Mate {
            exp: -1,
            dead: 0xf2,
            level: 0xfe,
        };
        expected.detail_payload = Some(update.raw_detail[1..].to_vec());
        assert_eq!(at, location(3));
        assert_eq!(changed, expected);
        assert_eq!(original.detail, crate::model::ItemDetail::None);
        let mut wrong = update.clone();
        wrong.item_id = 999;
        assert_eq!(
            project_native_item_detail_update(&wrong, context),
            Some(NativeItemTaskMutation::None)
        );
        assert_eq!(
            project_native_item_detail_update(&update, NativeItemTaskContext::Missing),
            Some(NativeItemTaskMutation::None)
        );
        assert!(
            project_native_item_detail_update(&update, NativeItemTaskContext::Unknown).is_none()
        );
        for container in [1, 3, 4, 0xfc] {
            wrong = update.clone();
            wrong.container = container;
            assert_eq!(
                project_native_item_detail_update(&wrong, NativeItemTaskContext::Unknown),
                Some(NativeItemTaskMutation::None)
            );
        }
        let empty = Item {
            id: 0,
            template_id: 0,
            ..Default::default()
        };
        wrong = update.clone();
        wrong.item_id = 0;
        assert_eq!(
            project_native_item_detail_update(
                &wrong,
                NativeItemTaskContext::Found {
                    location: location(3),
                    item: &empty
                }
            ),
            Some(NativeItemTaskMutation::None)
        );
        let valid_zero = Item {
            id: 0,
            template_id: 8001,
            ..Default::default()
        };
        assert!(matches!(
            project_native_item_detail_update(
                &wrong,
                NativeItemTaskContext::Found {
                    location: location(3),
                    item: &valid_zero
                }
            ),
            Some(NativeItemTaskMutation::Replace { .. })
        ));
        wrong = update.clone();
        wrong.raw_detail.pop();
        assert!(project_native_item_detail_update(&wrong, context).is_none());
        let rows = [None, None, None, Some(&original)];
        let inventory = NativeItemTaskInventory {
            equipment: NativeItemTaskStorage::Unknown,
            bag: NativeItemTaskStorage::Present {
                records: &rows,
                extent: 4,
                container: 2,
            },
            bank: NativeItemTaskStorage::Unknown,
        };
        assert!(
            matches!(lookup_native_item_detail_update(&update,&inventory),NativeItemTaskContext::Found{item,..} if item.id==123)
        );
    }
}
