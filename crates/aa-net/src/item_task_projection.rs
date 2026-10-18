//! Native Count/Remove task outputs, given source-exact pre-row lookup context.
//!
//! This does not look up items, update `Inventory`, dispatch observers, or retain history.
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
}
