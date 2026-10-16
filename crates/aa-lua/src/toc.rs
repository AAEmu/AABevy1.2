//! `toc.g` parsing: the ordered script list of one UI module.
//!
//! Format: one path per line; `#` lines (`## author : ...`) are comments, blank lines are ignored,
//! a UTF-8 BOM may precede the first line. Paths are relative to the toc's directory
//! (`locale/en_us.lua`, `../baselib.lua`) or start with `SCRIPTS/`, the root of the UI tree the
//! toc lives in (`scriptsbin/x2ui/` or `scriptsbin/globalui/`). `.lua` names map to the compiled
//! `.alb` in the pak (the client does the same when the `lua_use_binary` CVar is 1).
//!
//! This mirrors `X2::GameClient::UIContentsManager::LoadContent` in `x2game.dll`: it finds
//! `<root>/<content>/toc.g`, then for each line replaces a leading `SCRIPTS/` by the root, else
//! prefixes the content directory, strips one directory per `../`, swaps `.lua` for `.alb` and
//! executes the file, logging `cannot load %s file.` and carrying on when one fails.

use anyhow::{Context, Result};

/// The default directory the `SCRIPTS/` prefix stands for (the in-game and login UI tree).
pub const SCRIPTS_ROOT: &str = "scriptsbin/x2ui";

/// `foo.lua` -> `foo.alb`; other names unchanged.
pub fn alb_path(path: &str) -> String {
    let p = path.replace('\\', "/");
    match p.len().checked_sub(4) {
        Some(i) if p[i..].eq_ignore_ascii_case(".lua") => format!("{}.alb", &p[..i]),
        _ => p,
    }
}

/// A CryEngine script path as passed to `Script.ReloadScript` (`SCRIPTS/x2ui/chat/option.lua`,
/// `scripts/x2ui/ucc_view.lua`) -> the compiled file in the pak (`scriptsbin/x2ui/chat/option.alb`).
pub fn script_path(path: &str) -> String {
    let p = aa_pak::normalize(path);
    let p = p.strip_prefix("game/").unwrap_or(&p);
    let p = match p.strip_prefix("scripts/") {
        Some(rest) => format!("scriptsbin/{rest}"),
        None => p.to_string(),
    };
    alb_path(&p)
}

/// The script entries of a toc's text, in order, as written.
pub fn parse_toc(text: &str) -> Vec<&str> {
    text.trim_start_matches('\u{feff}')
        .lines()
        .map(|l| l.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}'))
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect()
}

/// Resolve one toc entry against the toc's directory to a normalised pak path
/// (lowercase, `/`, relative to `game/`, `.lua` -> `.alb`).
pub fn resolve(toc_dir: &str, entry: &str) -> String {
    let entry = entry.replace('\\', "/");
    let joined = match entry.get(..8) {
        Some(p) if p.eq_ignore_ascii_case("scripts/") => format!("{}/{}", scripts_root(toc_dir), &entry[8..]),
        _ => format!("{toc_dir}/{entry}"),
    };
    let mut parts: Vec<&str> = Vec::new();
    for part in joined.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    alb_path(&aa_pak::normalize(&parts.join("/")))
}

/// `scriptsbin/<tree>` for a directory inside it (`scriptsbin/x2ui/hud` -> `scriptsbin/x2ui`).
fn scripts_root(dir: &str) -> String {
    let d = aa_pak::normalize(dir);
    let mut it = d.split('/');
    match (it.next(), it.next()) {
        (Some("scriptsbin"), Some(tree)) if !tree.is_empty() => format!("scriptsbin/{tree}"),
        _ => SCRIPTS_ROOT.to_string(),
    }
}

/// The scripts a `toc.g` loads, as pak paths (`scriptsbin/x2ui/loginstage/baselib.alb`), in load
/// order. `toc_path` is a pak path such as `scriptsbin/x2ui/hud/toc.g` (a `game/` prefix is fine).
pub fn toc_files(toc_path: &str) -> Result<Vec<String>> {
    let text = aa_pak::Pak::global().read_string(toc_path).with_context(|| format!("reading {toc_path}"))?;
    Ok(toc_files_from(toc_path, &text))
}

/// [`toc_files`] for toc text already in memory.
pub fn toc_files_from(toc_path: &str, text: &str) -> Vec<String> {
    let norm = aa_pak::normalize(toc_path);
    let norm = norm.strip_prefix("game/").unwrap_or(&norm);
    let dir = norm.rsplit_once('/').map_or("", |(d, _)| d);
    parse_toc(text).into_iter().map(|e| resolve(dir, e)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_relative_and_scripts_paths() {
        let toc = "\u{feff}## author : prodg\r\n\r\n../baselib.lua\r\nlocale/en_us.lua\r\n  SCRIPTS/customizing/cosmetic.lua \r\n";
        let files = toc_files_from("game/scriptsbin/x2ui/loginstage/character_create/toc.g", toc);
        assert_eq!(
            files,
            [
                "scriptsbin/x2ui/loginstage/baselib.alb",
                "scriptsbin/x2ui/loginstage/character_create/locale/en_us.alb",
                "scriptsbin/x2ui/customizing/cosmetic.alb",
            ]
        );
    }

    #[test]
    fn globalui_scripts_root() {
        let files = toc_files_from("scriptsbin/globalui/loading/toc.g", "SCRIPTS/loading/a.lua\nb.lua");
        assert_eq!(files, ["scriptsbin/globalui/loading/a.alb", "scriptsbin/globalui/loading/b.alb"]);
    }

    #[test]
    fn script_paths() {
        assert_eq!(script_path("SCRIPTS/x2ui/baselib/common_view.lua"), "scriptsbin/x2ui/baselib/common_view.alb");
        assert_eq!(script_path("scripts/x2ui/ucc_view.lua"), "scriptsbin/x2ui/ucc_view.alb");
        assert_eq!(script_path("scripts/Utils/Math.lua"), "scriptsbin/utils/math.alb");
    }

    #[test]
    fn alb_names() {
        assert_eq!(alb_path("a/b.LUA"), "a/b.alb");
        assert_eq!(alb_path("a/b.alb"), "a/b.alb");
        assert_eq!(alb_path("toc.g"), "toc.g");
    }
}
