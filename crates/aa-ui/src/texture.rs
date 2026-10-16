//! UI textures: path resolution (locale overrides), sizes, and `.g` texture-info files.
//!
//! Scripts name textures like `ui/login_stage/img_text.dds`. The client prefers a locale copy
//! (`ui/login_stage/en_us/img_text.dds`) when one exists. Texture info (`GetTextureData`) comes
//! from the `.g` file beside the texture: an indented tree of named entries such as
//!
//! ```text
//! credits_df
//!     coords ( 243, 0, 107, 35 )
//! ```

use std::collections::HashMap;

/// One node of a `.g` file.
#[derive(Clone, Debug, Default)]
pub struct GNode {
    pub name: String,
    /// Values of `name ( a, b, c )` or `name value`.
    pub args: Vec<String>,
    pub children: Vec<GNode>,
}

impl GNode {
    pub fn child(&self, name: &str) -> Option<&GNode> {
        self.children
            .iter()
            .find(|c| c.name.eq_ignore_ascii_case(name))
    }
    pub fn nums(&self) -> Vec<f32> {
        self.args
            .iter()
            .filter_map(|a| a.trim().parse().ok())
            .collect()
    }
}

/// Parse a `.g` file into its top-level nodes.
pub fn parse_g(text: &str) -> Vec<GNode> {
    // Stack of (indent, node).
    let mut root = GNode::default();
    let mut stack: Vec<(usize, GNode)> = vec![(0, std::mem::take(&mut root))];
    stack[0].0 = usize::MAX; // sentinel: never popped by indentation
    for raw in text.lines() {
        let line = raw.trim_end().trim_start_matches('\u{feff}');
        let content = line.trim_start();
        if content.is_empty() || content.starts_with("//") || content.starts_with('#') {
            continue;
        }
        let indent = line.len() - content.len();
        let node = parse_line(content);
        while stack.len() > 1 && stack.last().unwrap().0 >= indent {
            let (_, n) = stack.pop().unwrap();
            stack.last_mut().unwrap().1.children.push(n);
        }
        stack.push((indent, node));
    }
    while stack.len() > 1 {
        let (_, n) = stack.pop().unwrap();
        stack.last_mut().unwrap().1.children.push(n);
    }
    stack.pop().unwrap().1.children
}

fn parse_line(s: &str) -> GNode {
    if let Some(open) = s.find('(') {
        let name = s[..open].trim().to_string();
        let inner = s[open + 1..].trim_end().trim_end_matches(')');
        let args = inner
            .split(',')
            .map(|a| a.trim().to_string())
            .filter(|a| !a.is_empty())
            .collect();
        GNode {
            name,
            args,
            children: Vec::new(),
        }
    } else {
        let mut it = s.splitn(2, char::is_whitespace);
        let name = it.next().unwrap_or("").to_string();
        let args = it
            .next()
            .map(|v| vec![v.trim().to_string()])
            .unwrap_or_default();
        GNode {
            name,
            args,
            children: Vec::new(),
        }
    }
}

/// Texture lookups shared by the Lua natives and the renderer.
pub struct TextureDb {
    pub locale: String,
    resolved: HashMap<String, Option<String>>,
    sizes: HashMap<String, Option<(u32, u32)>>,
    g_files: HashMap<String, Vec<GNode>>,
}

impl TextureDb {
    pub fn new(locale: &str) -> TextureDb {
        TextureDb {
            locale: locale.to_string(),
            resolved: HashMap::new(),
            sizes: HashMap::new(),
            g_files: HashMap::new(),
        }
    }

    /// The pak path a script texture path loads: locale copy first, `.dds` when no extension.
    pub fn resolve(&mut self, path: &str) -> Option<String> {
        if let Some(r) = self.resolved.get(path) {
            return r.clone();
        }
        let pak = aa_pak::Pak::global();
        let mut p = aa_pak::normalize(path);
        if let Some(s) = p.strip_prefix("game/") {
            p = s.to_string();
        }
        let has_ext = p.rsplit('/').next().is_some_and(|f| f.contains('.'));
        let mut candidates = Vec::new();
        let with_ext = if has_ext {
            p.clone()
        } else {
            format!("{p}.dds")
        };
        if let Some((dir, file)) = with_ext.rsplit_once('/') {
            candidates.push(format!("{dir}/{}/{file}", self.locale));
        }
        candidates.push(with_ext.clone());
        // Some scripts name .tga/.png art that ships as .dds.
        if let Some(stem) = with_ext.rsplit_once('.').map(|(s, _)| s) {
            candidates.push(format!("{stem}.dds"));
        }
        let r = candidates.into_iter().find(|c| pak.exists(c));
        if r.is_none() {
            bevy::log::warn!("aa-ui: texture not found: {path}");
        }
        self.resolved.insert(path.to_string(), r.clone());
        r
    }

    /// Pixel size from the DDS header.
    pub fn size(&mut self, path: &str) -> Option<(u32, u32)> {
        let resolved = self.resolve(path)?;
        if let Some(s) = self.sizes.get(&resolved) {
            return *s;
        }
        let s = aa_pak::Pak::global().read(&resolved).ok().and_then(|b| {
            if b.len() >= 20 && &b[0..4] == b"DDS " {
                let h = u32::from_le_bytes(b[12..16].try_into().unwrap());
                let w = u32::from_le_bytes(b[16..20].try_into().unwrap());
                Some((w, h))
            } else {
                None
            }
        });
        self.sizes.insert(resolved, s);
        s
    }

    /// `.g` entry `key` for a texture (`UIParent:GetTextureData(path, key)`).
    pub fn info(&mut self, path: &str, key: &str) -> Option<GNode> {
        let g = self.g_path(path)?;
        if !self.g_files.contains_key(&g) {
            let nodes = aa_pak::Pak::global()
                .read_string(&g)
                .map(|t| parse_g(&t))
                .unwrap_or_default();
            self.g_files.insert(g.clone(), nodes);
        }
        self.g_files[&g]
            .iter()
            .find(|n| n.name == key)
            .or_else(|| {
                self.g_files[&g]
                    .iter()
                    .find(|n| n.name.eq_ignore_ascii_case(key))
            })
            .cloned()
    }

    fn g_path(&mut self, path: &str) -> Option<String> {
        let tex = self
            .resolve(path)
            .unwrap_or_else(|| aa_pak::normalize(path));
        let stem = tex
            .rsplit_once('.')
            .map(|(s, _)| s.to_string())
            .unwrap_or(tex);
        let g = format!("{stem}.g");
        if aa_pak::Pak::global().exists(&g) {
            return Some(g);
        }
        // Locale texture without a locale .g: try the base .g.
        let base = g.replace(&format!("/{}/", self.locale), "/");
        aa_pak::Pak::global().exists(&base).then_some(base)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_g() {
        let n = parse_g(
            "credits_df\n    coords ( 243, 0, 107, 35 )\ncredits_ov\n    coords ( 243, 35, 107, 35 )\n    extent ( 10, 20 )\n",
        );
        assert_eq!(n.len(), 2);
        assert_eq!(n[0].name, "credits_df");
        assert_eq!(
            n[0].child("coords").unwrap().nums(),
            vec![243.0, 0.0, 107.0, 35.0]
        );
        assert_eq!(n[1].child("extent").unwrap().nums(), vec![10.0, 20.0]);
    }
}
