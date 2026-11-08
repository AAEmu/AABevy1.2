//! `X2Book.GetBookInfo` and the original book window.
//!
//! `DoodadFuncOpenPaper` raises `OPEN_PAPER("page", id)` / `("book", id)`; `webbrowser/book.alb`
//! asks `X2Book:GetBookInfo(book_id)` for the book's `name` and its `pages` (the
//! `book_elems.book_page_id` ids in row order). The pages themselves are HTML from
//! `book_page_contents`; the original window requests them from the embedder
//! (`webBrowser:RequestBookPage`), which this client does not render yet.

use std::collections::HashMap;

use mlua::{Lua, Table};
use rusqlite::Connection;

use crate::hud::data::any_int;

#[derive(Default)]
pub(super) struct Tables {
    /// `books.id` -> (localized name, page ids in `book_elems` order).
    books: HashMap<u32, (String, Vec<u32>)>,
}

impl Tables {
    pub(super) fn load(conn: &Connection) -> rusqlite::Result<Self> {
        let mut localized = HashMap::new();
        let mut q = conn.prepare("SELECT idx, en_us FROM localized_texts WHERE tbl_name = 'books' AND tbl_column_name = 'name' AND en_us IS NOT NULL")?;
        for row in q.query_map([], |r| Ok((any_int(r, 0)? as u32, r.get::<_, String>(1)?)))? {
            let (id, text) = row?;
            localized.insert(id, text);
        }
        let mut books = HashMap::new();
        let mut q = conn.prepare("SELECT id, name FROM books")?;
        for row in q.query_map([], |r| {
            Ok((
                any_int(r, 0)? as u32,
                r.get::<_, Option<String>>(1)?.unwrap_or_default(),
            ))
        })? {
            let (id, raw) = row?;
            let name = localized
                .remove(&id)
                .filter(|s| !s.is_empty())
                .unwrap_or(raw);
            books.insert(id, (name, Vec::new()));
        }
        if let Ok(mut q) = conn.prepare("SELECT book_id, book_page_id FROM book_elems ORDER BY id")
        {
            for row in q.query_map([], |r| Ok((any_int(r, 0)? as u32, any_int(r, 1)? as u32)))? {
                let (book, page) = row?;
                if let Some((_, pages)) = books.get_mut(&book) {
                    pages.push(page);
                }
            }
        }
        Ok(Self { books })
    }

    pub(super) fn info(&self, book: u32) -> Option<(&str, &[u32])> {
        self.books
            .get(&book)
            .map(|(name, pages)| (name.as_str(), pages.as_slice()))
    }
}

/// `X2Book.GetBookInfo(book_id)` -> `{ name, pages }` (`book.alb` main.F4).
pub(super) fn install(lua: &Lua, bridge: &crate::hud::api::Shared) -> mlua::Result<()> {
    let book: Table = lua.globals().get("X2Book")?;
    let state = bridge.clone();
    book.set(
        "GetBookInfo",
        lua.create_function(move |lua, (_this, id): (mlua::Value, u32)| {
            let data = state.borrow();
            let Some((name, pages)) = data.game.get().and_then(|g| g.books.info(id)) else {
                return Ok(None);
            };
            let t = lua.create_table()?;
            t.set("name", name)?;
            let list = lua.create_table()?;
            for (i, page) in pages.iter().enumerate() {
                list.set(i + 1, *page)?;
            }
            t.set("pages", list)?;
            Ok(Some(t))
        })?,
    )?;
    Ok(())
}
