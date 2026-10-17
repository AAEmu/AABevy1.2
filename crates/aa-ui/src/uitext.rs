//! `X2Locale:LocalizeUiText(category, key, ...)` from the game database: the `ui_texts` table
//! (`category_id`, `key`, `text`) with the locale's column of `localized_texts` when present.
//! `$1`, `$2`, ... in a text are replaced by the extra arguments. Tables are loaded once per
//! (database, locale) and shared by every UI runtime (each stage builds a new one).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

/// (category id, key) -> text.
pub type UiTexts = HashMap<(i64, String), String>;

/// Texts per (database, locale).
type Cache = Mutex<HashMap<(PathBuf, String), Arc<UiTexts>>>;

fn cache() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// The `localized_texts` column for a client locale (`en_us` falls back to the base text).
fn locale_column(locale: &str) -> Option<&'static str> {
    Some(match locale {
        "ko" => "ko",
        "en_us" => "en_us",
        "zh_cn" => "zh_cn",
        "zh_tw" => "zh_tw",
        "ja" => "ja",
        "ru" => "ru",
        "de" => "de",
        "fr" => "fr",
        _ => return None,
    })
}

/// Load (or reuse) the UI texts of `locale` from the database at `path` (opened read-only).
pub fn load(path: &Path, locale: &str) -> anyhow::Result<Arc<UiTexts>> {
    let key = (path.to_path_buf(), locale.to_string());
    if let Some(t) = cache().lock().unwrap().get(&key) {
        return Ok(t.clone());
    }
    let flags =
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let db = rusqlite::Connection::open_with_flags(path, flags)?;
    let has_localized = db
        .prepare("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'localized_texts'")?
        .exists([])?;
    let sql = match (has_localized, locale_column(locale)) {
        (true, Some(col)) => format!(
            "SELECT u.category_id, u.key, COALESCE(l.{col}, u.text) FROM ui_texts u \
             LEFT JOIN localized_texts l ON l.tbl_name = 'ui_texts' \
             AND l.tbl_column_name = 'text' AND l.idx = u.id"
        ),
        _ => "SELECT category_id, key, text FROM ui_texts".to_string(),
    };
    let mut out = UiTexts::new();
    let mut st = db.prepare(&sql)?;
    let rows = st.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<String>>(2)?,
        ))
    })?;
    for (cat, key, text) in rows.flatten() {
        if let Some(t) = text {
            out.insert((cat, key), t);
        }
    }
    let t = Arc::new(out);
    cache().lock().unwrap().insert(key, t.clone());
    Ok(t)
}

/// The text for (category, key) with `$1`.. replaced by `args`; the key when there is none.
pub fn localize(texts: &UiTexts, category: i64, key: &str, args: &[String]) -> String {
    let Some(s) = texts.get(&(category, key.to_string())) else {
        return key.to_string();
    };
    if args.is_empty() || !s.contains('$') {
        return s.clone();
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '$'
            && let Some(d) = chars.peek().and_then(|d| d.to_digit(10))
        {
            chars.next();
            if let Some(a) = (d as usize).checked_sub(1).and_then(|i| args.get(i)) {
                out.push_str(a);
            }
            continue;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders() {
        let mut t = UiTexts::new();
        t.insert((1, "k".into()), "Level $1 of $2".into());
        assert_eq!(
            localize(&t, 1, "k", &["3".into(), "50".into()]),
            "Level 3 of 50"
        );
        assert_eq!(localize(&t, 1, "k", &[]), "Level $1 of $2");
        assert_eq!(localize(&t, 2, "missing", &[]), "missing");
    }
}
