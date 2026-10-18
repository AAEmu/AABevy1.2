use super::{View, catalog::Tables};
use std::cmp::Ordering;

pub(super) fn sub_count(main: i32) -> i32 {
    match main {
        1 | 2 => 4,
        3 => 5,
        4 | 5 => 3,
        _ => 0,
    }
}

///3900c9b0: same-parent rows compare their own priority; otherwise compare parent descriptors.
pub(super) fn category_order(tables: &Tables, a: u32, b: u32) -> Ordering {
    let (Some(left), Some(right)) = (tables.get(a), tables.get(b)) else {
        return a.cmp(&b);
    };
    let (left, right) = if left.parent == right.parent {
        (left, right)
    } else {
        (
            if left.parent != 0 {
                tables.get(left.parent).unwrap_or(left)
            } else {
                left
            },
            if right.parent != 0 {
                tables.get(right.parent).unwrap_or(right)
            } else {
                right
            },
        )
    };
    left.priority.cmp(&right.priority).then_with(|| a.cmp(&b))
}

pub(super) fn list(tables: &Tables, view: &View, main: i32, sub: i32, filter: i32) -> mlua::Result<Vec<u32>> {
    let progress = |id| view.cache.get(id).copied().unwrap_or_default();
    if main == 0 && sub == 0 && filter == 5 {
        return Ok(view.tracing.iter().copied().filter(|&id| id != 0).collect());
    }
    let mut ids = if main == 0 && sub == 0 && filter == 2 {
        view.cache
            .entries
            .values()
            .filter(|row| row.completed_at != 0)
            .map(|row| row.id)
            .collect::<Vec<_>>()
    } else if sub > 0 && sub <= sub_count(main) && (1..=4).contains(&filter) {
        tables
            .rows
            .iter()
            .filter(|entry| {
                let row = progress(entry.id);
                entry.main == main
                    && entry.sub == sub
                    && entry.active
                    && (!entry.hidden || row.completed_at != 0)
                    && match filter {
                        1 => true,
                        2 => row.completed_at != 0,
                        3 => row.completed_at == 0,
                        4 => row.completed_at == 0 && row.amount > 0,
                        _ => false,
                    }
            })
            .map(|entry| entry.id)
            .collect()
    } else {
        return Ok(Vec::new());
    };
    if filter == 4 && ids.iter().any(|&id| tables.get(id).unwrap().rule.completed_amount() == 0) {
        // Native's non-strict zero-denominator comparator is outside the real1743-row catalog.
        return Err(mlua::Error::runtime(
            "achievement progress sorting requires a nonzero catalog threshold",
        ));
    }
    ids.sort_unstable_by(|&a, &b| match filter {
        2 => (progress(b).completed_at as u64)
            .cmp(&(progress(a).completed_at as u64))
            .then_with(|| a.cmp(&b)),
        4 => {
            let ratio = |id| progress(id).amount as f32 / tables.get(id).unwrap().rule.completed_amount() as f32;
            ratio(b)
                .partial_cmp(&ratio(a))
                .unwrap_or(Ordering::Equal)
                .then_with(|| category_order(tables, a, b))
        }
        _ => category_order(tables, a, b),
    });
    Ok(ids)
}
