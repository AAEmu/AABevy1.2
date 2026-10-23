//! Executed original x86 fixtures; no ignored artifacts or RE dependencies at test time.
use crate::{
    item_task_projection::{
        NativeItemTaskContext, NativeItemTaskLocation, NativeItemTaskMutation,
        project_native_item_task,
    },
    items::{ItemTask, ItemTasks},
    model::{Item, ItemDetail},
    wire::Reader,
};

fn unhex(value: &str) -> Vec<u8> {
    value
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

pub(crate) fn goldens() -> impl Iterator<Item = (&'static str, Vec<u8>, ItemTask, Item)> {
    include_str!("../tests/fixtures/music-sheet-detail/native.tsv")
        .lines()
        .filter(|line| !line.starts_with('#'))
        .map(|line| {
            let fields: Vec<_> = line.split('\t').collect();
            let raw = unhex(fields[1]);
            let mut body = vec![3, 1];
            body.extend_from_slice(&raw);
            body.push(0);
            let mut reader = Reader::new(&body);
            let changes = ItemTasks::read(&mut reader).unwrap();
            assert!(!changes.truncated && reader.is_empty());
            let expected_wire = unhex(fields[2]);
            let mut reader = Reader::new(&expected_wire);
            let expected = Item::read(&mut reader).unwrap().unwrap();
            assert!(reader.is_empty());
            (
                fields[0],
                raw,
                changes.tasks.into_iter().next().unwrap(),
                expected,
            )
        })
}

fn detail_parts(
    task: &mut ItemTask,
) -> (&mut ItemDetail, &mut Option<Vec<u8>>, &mut Option<Vec<u8>>) {
    match task {
        ItemTask::Add {
            item,
            raw_detail_block,
            ..
        } => (&mut item.detail, &mut item.detail_payload, raw_detail_block),
        ItemTask::Update {
            detail,
            detail_payload,
            raw_detail_block,
            ..
        } => (detail, detail_payload, raw_detail_block),
        _ => panic!("not a padded detail row"),
    }
}

fn original() -> Item {
    Item {
        id: 0x1122334455667788,
        template_id: 8001,
        grade: 6,
        flags: 2,
        count: 5,
        made_unit_id: 47,
        create_time: -7,
        lifespan_mins: 11,
        world_id: 7,
        unsecure_time: 12,
        unpack_time: -13,
        ..Default::default()
    }
}

#[test]
fn music_sheet_native_goldens_copy_add_take_owner_and_update_after_mutation() {
    let old = original();
    let location = NativeItemTaskLocation {
        container: 2,
        slot: 3,
    };
    let context = NativeItemTaskContext::Found {
        location,
        item: &old,
    };
    for (name, _, mut task, expected) in goldens() {
        let (_, payload, raw) = detail_parts(&mut task);
        let block = raw.as_ref().unwrap().clone();
        assert_eq!(block.len(), 128);
        assert_eq!(*payload, Some(block[1..9].to_vec()));
        assert!(block[9..].iter().any(|b| *b != 0));
        let retained = task.clone();
        let result = project_native_item_task(&task, context).unwrap();
        assert!(result.admitted, "{name}");
        assert_eq!(result.output, Some(expected.clone()), "{name}");
        assert_eq!(
            result.mutation,
            NativeItemTaskMutation::Replace {
                location,
                item: expected
            }
        );
        assert_eq!(
            task, retained,
            "complete raw128/reserved bytes survive projection"
        );
        assert!(project_native_item_task(&task, NativeItemTaskContext::Unknown).is_none());
        let missing = project_native_item_task(&task, NativeItemTaskContext::Missing).unwrap();
        assert!(!missing.admitted && missing.output.is_none());
        assert_eq!(missing.mutation, NativeItemTaskMutation::None);
        assert!(
            project_native_item_task(
                &task,
                NativeItemTaskContext::Found {
                    location: NativeItemTaskLocation {
                        container: 2,
                        slot: 2
                    },
                    item: &old,
                }
            )
            .is_none()
        );
        // Update9 copies the detail even over a known native empty template;
        // packetID999 is deliberately unrelated to the physical PRE-row ID.
        if matches!(task, ItemTask::Update { .. }) {
            let empty = Item::default();
            let result = project_native_item_task(
                &task,
                NativeItemTaskContext::Found {
                    location,
                    item: &empty,
                },
            )
            .unwrap();
            assert!(result.admitted);
            assert_eq!(result.output.unwrap().template_id, 0);
        }
    }
    assert_eq!(old, original(), "borrowed PRE-row is unchanged");
}

#[test]
fn music_sheet_refuses_every_short_raw_union_and_parsed_prefix_disagreement() {
    let old = original();
    let context = NativeItemTaskContext::Found {
        location: NativeItemTaskLocation {
            container: 2,
            slot: 3,
        },
        item: &old,
    };
    for (name, _, task, _) in goldens() {
        for len in (0..128).chain([129, 256]) {
            let mut malformed = task.clone();
            let (_, _, raw) = detail_parts(&mut malformed);
            raw.as_mut().unwrap().resize(len, 0xa5);
            assert!(
                project_native_item_task(&malformed, context).is_none(),
                "{name} len{len}"
            );
        }
        for mismatch in 0..8 {
            let mut bad = task.clone();
            let (detail, payload, raw) = detail_parts(&mut bad);
            match mismatch {
                0 => *raw = None,
                1 => raw.as_mut().unwrap()[0] = 0,
                2 => *detail = ItemDetail::Other(9),
                3 => *payload = None,
                4 => {
                    payload.as_mut().unwrap().pop();
                }
                5 => payload.as_mut().unwrap().push(0),
                6 => payload.as_mut().unwrap()[0] ^= 1,
                _ => raw.as_mut().unwrap()[8] ^= 1,
            }
            assert!(
                project_native_item_task(&bad, context).is_none(),
                "{name} mismatch{mismatch}"
            );
        }
        // Other already-decoded opaque tags remain unsupported even with a
        // valid raw128 and an exactly matching parsed payload of their size.
        for (tag, size) in [(4, 9), (5, 24), (6, 16), (7, 16), (9, 4), (255, 0)] {
            let mut unsupported = task.clone();
            let (detail, payload, raw) = detail_parts(&mut unsupported);
            *detail = ItemDetail::Other(tag);
            let block = raw.as_mut().unwrap();
            block[0] = tag;
            *payload = Some(block[1..1 + size].to_vec());
            assert!(
                project_native_item_task(&unsupported, context).is_none(),
                "{name} tag{tag}"
            );
        }
    }
}

#[test]
fn music_sheet_all_wire_prefixes_refuse_partial_rows_and_preserve_following_task_alignment() {
    for (name, raw, task, _) in goldens() {
        for end in 0..raw.len() {
            let mut prefix = vec![3, 1];
            prefix.extend_from_slice(&raw[..end]);
            let changes = ItemTasks::read(&mut Reader::new(&prefix)).unwrap();
            assert!(
                changes.truncated && changes.tasks.is_empty(),
                "{name} wire prefix{end}"
            );
        }
        let mut full = vec![3, 2];
        full.extend_from_slice(&raw);
        full.extend_from_slice(&[1, 17, 0, 0, 0, 0]); // Money17, force-remove count0
        let mut reader = Reader::new(&full);
        let changes = ItemTasks::read(&mut reader).unwrap();
        assert!(!changes.truncated && reader.is_empty(), "{name}");
        assert_eq!(changes.tasks, vec![task, ItemTask::Money(17)]);
    }
}
