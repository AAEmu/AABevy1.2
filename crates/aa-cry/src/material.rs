//! CryEngine `.mtl` material files (plain XML in the 1.2 pak) as plain data.
//!
//! ```xml
//! <Material MtlFlags="256">
//!  <SubMaterials>
//!   <Material Name="skin" Shader="HumanSkin" MtlFlags="524416" GenMask="a002" AlphaTest="0.5"
//!             Diffuse="0.18,0.16,0.14" Specular="0.16,0.47,0.5" Emissive="0,0,0"
//!             Shininess="41.5" Opacity="1" SpecularLevel="1" SurfaceType="" sRGB="2">
//!    <Textures>
//!     <Texture Map="Diffuse" File="game/objects/.../el_f_nude_df.dds"><TexMod/></Texture>
//!    </Textures>
//!    <PublicParams Melanin="0.75" .../>
//!   </Material>
//!  </SubMaterials>
//! </Material>
//! ```
//!
//! A mesh subset's material id indexes [`Material::sub_materials`] (or the root material when
//! it has none, see [`Material::sub`]).

use crate::{Error, Result};
use glam::Vec3;
use roxmltree::Node;

/// `MtlFlags` bits (CryEngine `EMaterialFlags`).
pub mod flags {
    pub const WIRE: u32 = 0x0001;
    pub const TWO_SIDED: u32 = 0x0002;
    pub const ADDITIVE: u32 = 0x0004;
    pub const DETAIL_DECAL: u32 = 0x0008;
    pub const LIGHTING: u32 = 0x0010;
    pub const NO_SHADOW: u32 = 0x0020;
    pub const ALWAYS_USED: u32 = 0x0040;
    pub const PURE_CHILD: u32 = 0x0080;
    pub const MULTI_SUBMTL: u32 = 0x0100;
    pub const NO_PHYSICALIZE: u32 = 0x0200;
    pub const NO_DRAW: u32 = 0x0400;
    pub const NO_PREVIEW: u32 = 0x0800;
    pub const NOT_INSTANCED: u32 = 0x1000;
    pub const COLLISION_PROXY: u32 = 0x2000;
    pub const SCATTER: u32 = 0x4000;
    pub const REQUIRE_FORWARD_RENDERING: u32 = 0x8000;
    pub const NON_REMOVABLE: u32 = 0x1_0000;
    pub const HIDE_ON_BREAK: u32 = 0x2_0000;
    pub const UI_MATERIAL: u32 = 0x4_0000;
    pub const SHADER_GEN_MASK_64BIT: u32 = 0x8_0000;
    pub const RAYCAST_PROXY: u32 = 0x10_0000;
    pub const REQUIRE_NEAREST_CUBEMAP: u32 = 0x20_0000;
}

/// The `Map` attribute of a `<Texture>`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TextureMap {
    Diffuse,
    Specular,
    /// `Normalmap` (tangent-space normal map, usually `_ddn`).
    Normal,
    /// `Bumpmap` (height / normal for some shaders).
    Bump,
    Environment,
    SubSurface,
    Opacity,
    Detail,
    Decal,
    /// `Custom`.
    Custom,
    /// `[1] Custom`.
    Custom1,
    /// Anything else, as written.
    Other(String),
}

/// `<TexMod>` texture coordinate modifier (only the parts that matter for rendering).
#[derive(Clone, Debug, PartialEq)]
pub struct TexMod {
    pub tile_u: f32,
    pub tile_v: f32,
    pub offset_u: f32,
    pub offset_v: f32,
    /// Rotation around W in degrees (`RotateW`).
    pub rotate_w: f32,
    /// TexMod attributes whose coordinate generation/animation is not implemented. Retained so
    /// static texture composition can decline them instead of silently baking the wrong UVs.
    pub unhandled: Vec<(String, String)>,
}

impl Default for TexMod {
    fn default() -> Self {
        TexMod {
            tile_u: 1.0,
            tile_v: 1.0,
            offset_u: 0.0,
            offset_v: 0.0,
            rotate_w: 0.0,
            unhandled: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Texture {
    pub map: TextureMap,
    /// As written (may start with `game/`, may use `\`, mixed case); resolve with
    /// [`Texture::pak_path`].
    pub file: String,
    /// `Amount` (normal map strength, 0..100+), if given.
    pub amount: Option<f32>,
    /// `None` when `<TexMod/>` is empty or absent.
    pub tex_mod: Option<TexMod>,
    /// Raw texture-level attributes other than `Map`, `File` and `Amount`.
    /// Names, values and presence are retained without interpreting sampler or source settings.
    pub attributes: Vec<(String, String)>,
}

impl Texture {
    /// Normalised pak path (lowercase, `/`, without a leading `game/`); `.tif`/`.tga` names are
    /// mapped to `.dds` as the engine does. Empty for an empty `File` (and for a `File` that names
    /// a directory). See [`texture_pak_path`].
    pub fn pak_path(&self) -> String {
        texture_pak_path(&self.file)
    }
}

/// One material (the root or a sub-material).
#[derive(Clone, Debug, PartialEq)]
pub struct Material {
    pub name: String,
    /// `Shader` (e.g. `Illum`, `Vegetation`, `Hair`, `HumanSkin`, `Cut`, `NoDraw`), as written.
    pub shader: String,
    /// `MtlFlags`, see [`flags`].
    pub flags: u32,
    /// `GenMask`, the shader generation mask. Written in hex when `MtlFlags` has
    /// [`flags::SHADER_GEN_MASK_64BIT`] (almost all character / item / env materials), in decimal
    /// (possibly negative, as a signed 32 bit value) otherwise; both are returned as the mask
    /// value (a negative decimal as its `u32` bit pattern).
    pub gen_mask: u64,
    /// `StringGenMask` (e.g. `%ALPHAGLOW%...`), if given.
    pub string_gen_mask: String,
    pub surface_type: String,
    /// Linear 0..1 colours as written.
    pub diffuse: Vec3,
    pub specular: Vec3,
    pub emissive: Vec3,
    pub shininess: f32,
    pub specular_level: f32,
    /// 1 = opaque; < 1 = alpha blended.
    pub opacity: f32,
    /// Alpha test threshold, 0 = none.
    pub alpha_test: f32,
    pub glow_amount: f32,
    /// `vertModifType` (vertex deformation, e.g. bending), 0 = none.
    pub vert_modif_type: u32,
    /// `<Textures>` in document order. Textures with an empty `File` are not kept.
    pub textures: Vec<Texture>,
    /// `<PublicParams>` attributes, as written.
    pub public_params: Vec<(String, String)>,
    /// `<SubMaterials>`, indexed by a mesh subset's material id. An empty `<MaterialRef/>` slot
    /// (a reference to another material by `Name`, which is not resolved) is kept as a default
    /// material carrying only that `name`, so the indices stay right.
    pub sub_materials: Vec<Material>,
}

impl Default for Material {
    fn default() -> Self {
        Material {
            name: String::new(),
            shader: String::new(),
            flags: 0,
            gen_mask: 0,
            string_gen_mask: String::new(),
            surface_type: String::new(),
            diffuse: Vec3::ONE,
            specular: Vec3::ZERO,
            emissive: Vec3::ZERO,
            shininess: 10.0,
            specular_level: 1.0,
            opacity: 1.0,
            alpha_test: 0.0,
            glow_amount: 0.0,
            vert_modif_type: 0,
            textures: Vec::new(),
            public_params: Vec::new(),
            sub_materials: Vec::new(),
        }
    }
}

impl Material {
    /// Parses `.mtl` XML text (tolerates a BOM, leading whitespace/garbage before `<`, and the
    /// sloppy XML some files contain).
    ///
    /// Strategy: parse the text as XML; when that fails, rewrite it into well-formed XML
    /// (see `repair_xml`) and parse again; a file without any XML element is read as a Wavefront
    /// `.mtl` (three such CryEdit exports exist). Attribute names are matched case-insensitively.
    pub fn parse(text: &str) -> Result<Material> {
        parse_tracked(text).map(|(m, _)| m)
    }

    /// Loads and parses a `.mtl` from the pak.
    pub fn load(path: &str) -> Result<Material> {
        let bytes = crate::read(path)?;
        let text = String::from_utf8_lossy(&bytes);
        Material::parse(&text)
    }

    /// The material used by a mesh subset with this material id: `sub_materials[id]`, or `self`
    /// when there are no sub-materials. `None` when out of range.
    pub fn sub(&self, id: usize) -> Option<&Material> {
        if self.sub_materials.is_empty() { (id == 0).then_some(self) } else { self.sub_materials.get(id) }
    }

    pub fn texture(&self, map: &TextureMap) -> Option<&Texture> {
        self.textures.iter().find(|t| &t.map == map && !t.file.is_empty())
    }

    pub fn public_param(&self, name: &str) -> Option<&str> {
        self.public_params.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    pub fn two_sided(&self) -> bool {
        self.flags & flags::TWO_SIDED != 0
    }

    /// Not rendered: `NoDraw` shader or the NODRAW flag.
    pub fn no_draw(&self) -> bool {
        self.flags & flags::NO_DRAW != 0 || self.shader.eq_ignore_ascii_case("nodraw")
    }
}

/// Pak path of a texture reference: lowercase, `/` separators, `//` collapsed, no leading `./`,
/// `/` or `game/` (`aa_pak::Pak` accepts paths with or without `game/`), and a source-image
/// extension replaced by `.dds` (the pak only holds compiled textures; `.tif` refs resolve only
/// that way, 1,932 of them). Replaced: `tif tiff tga png jpg jpeg bmp psd` and the typos `dd`
/// and `ddn`; a name without extension gets `.dds`. Empty for an empty path and for a path that
/// names a directory (trailing `/`), i.e. "no texture".
///
/// Pure string function: it does not look in the pak, so the result may still not exist (some
/// textures are simply missing from the pak). The mapping is net positive but not perfect: 8
/// refs to `.../presets/materialtypes/.../rocky_diff.tga` (editor presets) exist as `.tga` and
/// have no `.dds`, while 2 `.tga` refs of real item / env materials exist only as `.dds`.
pub fn texture_pak_path(file: &str) -> String {
    let mut path = clean_path(file);
    if path.is_empty() || path.ends_with('/') {
        return String::new();
    }
    let name_at = path.rfind('/').map_or(0, |i| i + 1);
    match path[name_at..].rfind('.') {
        Some(dot) => {
            let ext_at = name_at + dot + 1;
            if SOURCE_IMAGE_EXTS.contains(&&path[ext_at..]) {
                path.truncate(ext_at);
                path.push_str("dds");
            }
        }
        None => path.push_str(".dds"),
    }
    path
}

/// Extensions of texture sources (and typos of `.dds`) that the compiled pak holds as `.dds`.
const SOURCE_IMAGE_EXTS: [&str; 10] = ["tif", "tiff", "tga", "png", "jpg", "jpeg", "bmp", "psd", "dd", "ddn"];

/// [`texture_pak_path`] without the extension mapping.
fn clean_path(file: &str) -> String {
    let mut path = file.trim().replace('\\', "/").to_ascii_lowercase();
    while path.contains("//") {
        path = path.replace("//", "/");
    }
    let mut rest = path.as_str();
    let mut game = false;
    loop {
        if let Some(r) = rest.strip_prefix("./").or_else(|| rest.strip_prefix('/')) {
            rest = r;
        } else if let (false, Some(r)) = (game, rest.strip_prefix("game/")) {
            rest = r;
            game = true;
        } else {
            break;
        }
    }
    rest.to_string()
}

// ---------------------------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------------------------

/// Sub-materials nested deeper than this are ignored (real files nest at most one level).
const MAX_DEPTH: usize = 32;

/// How [`parse_tracked`] got a document (for the whole-pak statistics).
#[derive(Clone, Debug, PartialEq, Eq)]
enum Route {
    /// The text was well-formed XML.
    Strict,
    /// The text needed `repair_xml`; carries the strict parser's complaint.
    Repaired(String),
    /// A Wavefront `.mtl` (no XML at all).
    Wavefront,
}

fn parse_tracked(text: &str) -> Result<(Material, Route)> {
    let text = text.trim_start_matches('\u{feff}');
    let why = match roxmltree::Document::parse(text) {
        Ok(doc) => return Ok((from_document(&doc)?, Route::Strict)),
        Err(e) => e.to_string(),
    };
    if let Some(xml) = repair_xml(text) {
        let doc = roxmltree::Document::parse(&xml)
            .map_err(|e| Error::invalid(format!("mtl xml: {e} (after repairing: {why})")))?;
        return Ok((from_document(&doc)?, Route::Repaired(why)));
    }
    if let Some(m) = parse_wavefront(text) {
        return Ok((m, Route::Wavefront));
    }
    Err(Error::invalid(format!("not a material file: {why}")))
}

fn is_tag(node: Node, name: &str) -> bool {
    node.is_element() && node.tag_name().name().eq_ignore_ascii_case(name)
}

fn from_document(doc: &roxmltree::Document) -> Result<Material> {
    let root = doc.root_element();
    let node = if is_tag(root, "Material") {
        root
    } else {
        root.descendants().find(|n| is_tag(*n, "Material")).ok_or_else(|| {
            Error::invalid(format!("no <Material> element (the root is <{}>)", root.tag_name().name()))
        })?
    };
    Ok(material(node, 0))
}

fn material(node: Node, depth: usize) -> Material {
    let mut m = Material::default();
    let mut gen_mask = None;
    for a in node.attributes() {
        let v = a.value();
        match a.name().to_ascii_lowercase().as_str() {
            "name" => m.name = v.to_string(),
            "shader" => m.shader = v.trim().to_string(),
            "mtlflags" => m.flags = parse_flags(v).unwrap_or(m.flags),
            "genmask" => gen_mask = Some(v),
            "stringgenmask" => m.string_gen_mask = v.trim().to_string(),
            "surfacetype" => m.surface_type = v.trim().to_string(),
            "diffuse" => m.diffuse = parse_color(v).unwrap_or(m.diffuse),
            "specular" => m.specular = parse_color(v).unwrap_or(m.specular),
            "emissive" => m.emissive = parse_color(v).unwrap_or(m.emissive),
            "shininess" => m.shininess = parse_f32(v).unwrap_or(m.shininess),
            "specularlevel" => m.specular_level = parse_f32(v).unwrap_or(m.specular_level),
            "opacity" => m.opacity = parse_f32(v).unwrap_or(m.opacity),
            "alphatest" => m.alpha_test = parse_f32(v).unwrap_or(m.alpha_test),
            "glowamount" => m.glow_amount = parse_f32(v).unwrap_or(m.glow_amount),
            "vertmodiftype" => m.vert_modif_type = v.trim().parse().unwrap_or(m.vert_modif_type),
            _ => {}
        }
    }
    // Needs MtlFlags, which may come after GenMask.
    if let Some(g) = gen_mask {
        m.gen_mask = parse_gen_mask(g, m.flags);
    }
    for child in node.children().filter(|c| c.is_element()) {
        match child.tag_name().name().to_ascii_lowercase().as_str() {
            "textures" => {
                let textures = child.children().filter(|t| is_tag(*t, "Texture"));
                m.textures.extend(textures.filter_map(texture));
            }
            "publicparams" => {
                let params = child.attributes();
                m.public_params.extend(params.map(|a| (a.name().to_string(), a.value().to_string())));
            }
            "submaterials" if depth < MAX_DEPTH => {
                for sub in child.children().filter(|s| s.is_element()) {
                    if is_tag(sub, "Material") {
                        m.sub_materials.push(material(sub, depth + 1));
                    } else if is_tag(sub, "MaterialRef") {
                        let name = sub.attributes().find(|a| a.name().eq_ignore_ascii_case("name"));
                        let name = name.map(|a| a.value().to_string()).unwrap_or_default();
                        m.sub_materials.push(Material { name, ..Material::default() });
                    }
                }
            }
            _ => {}
        }
    }
    m
}

/// `None` for a texture without a file.
fn texture(node: Node) -> Option<Texture> {
    let (mut map, mut file, mut amount) = (TextureMap::Other(String::new()), "", None);
    let mut attributes = Vec::new();
    for a in node.attributes() {
        match a.name().to_ascii_lowercase().as_str() {
            "map" => map = parse_texture_map(a.value()),
            "file" => file = a.value().trim(),
            "amount" => amount = parse_f32(a.value()),
            _ => attributes.push((a.name().to_string(), a.value().to_string())),
        }
    }
    if file.is_empty() {
        return None;
    }
    let tex_mod = node.children().find(|c| is_tag(*c, "TexMod")).and_then(tex_mod);
    Some(Texture { map, file: file.to_string(), amount, tex_mod, attributes })
}

fn parse_texture_map(v: &str) -> TextureMap {
    let key: String = v.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_ascii_lowercase();
    match key.as_str() {
        "diffuse" => TextureMap::Diffuse,
        "specular" => TextureMap::Specular,
        "normalmap" | "normal" => TextureMap::Normal,
        "bumpmap" | "bump" => TextureMap::Bump,
        "environment" => TextureMap::Environment,
        "subsurface" => TextureMap::SubSurface,
        "opacity" => TextureMap::Opacity,
        "detail" => TextureMap::Detail,
        "decal" => TextureMap::Decal,
        "custom" => TextureMap::Custom,
        "[1]custom" => TextureMap::Custom1,
        _ => TextureMap::Other(v.to_string()),
    }
}

/// `None` when every value is the default and no unhandled attributes are present.
fn tex_mod(node: Node) -> Option<TexMod> {
    let mut t = TexMod::default();
    for a in node.attributes() {
        match (a.name().to_ascii_lowercase().as_str(), parse_f32(a.value())) {
            ("tileu", Some(v)) => t.tile_u = v,
            ("tilev", Some(v)) => t.tile_v = v,
            ("offsetu", Some(v)) => t.offset_u = v,
            ("offsetv", Some(v)) => t.offset_v = v,
            ("rotatew", Some(v)) => t.rotate_w = v,
            ("tileu" | "tilev" | "offsetu" | "offsetv" | "rotatew", None) => {}
            _ => t
                .unhandled
                .push((a.name().to_string(), a.value().to_string())),
        }
    }
    (t != TexMod::default()).then_some(t)
}

fn parse_f32(v: &str) -> Option<f32> {
    v.trim().parse::<f32>().ok().filter(|f| f.is_finite())
}

/// `"r,g,b"` (also space separated, extra components ignored); `None` unless three numbers.
fn parse_color(v: &str) -> Option<Vec3> {
    let mut parts = v.split(|c: char| c == ',' || c.is_whitespace()).filter(|p| !p.is_empty());
    let r = parse_f32(parts.next()?)?;
    let g = parse_f32(parts.next()?)?;
    let b = parse_f32(parts.next()?)?;
    Some(Vec3::new(r, g, b))
}

/// Decimal `u32` (a negative value or `0x` hex is accepted too).
fn parse_flags(v: &str) -> Option<u32> {
    let v = v.trim();
    if let Ok(f) = v.parse::<u32>() {
        return Some(f);
    }
    if let Ok(f) = v.parse::<i64>() {
        return Some(f as u32);
    }
    u32::from_str_radix(v.strip_prefix("0x").or_else(|| v.strip_prefix("0X"))?, 16).ok()
}

/// Hex with [`flags::SHADER_GEN_MASK_64BIT`], else the legacy decimal 32 bit mask (which can be
/// negative). In the pak, 100% of hex-letter masks have the flag and 100% of negative ones lack
/// it; digit-only masks without the flag are decimal (`16777216` == hex `1000000`).
fn parse_gen_mask(v: &str, mtl_flags: u32) -> u64 {
    let v = v.trim();
    if mtl_flags & flags::SHADER_GEN_MASK_64BIT == 0 {
        if let Ok(m) = v.parse::<u64>() {
            return m;
        }
        if let Ok(m) = v.parse::<i32>() {
            return m as u32 as u64;
        }
    }
    let hex = v.strip_prefix("0x").or_else(|| v.strip_prefix("0X")).unwrap_or(v);
    u64::from_str_radix(hex, 16).unwrap_or(0)
}

// ---------------------------------------------------------------------------------------------
// Sloppy XML
// ---------------------------------------------------------------------------------------------

/// Deepest element nesting the repaired XML gets (roxmltree allows 1024).
const MAX_NEST: usize = 200;

/// Rewrites sloppy `.mtl` text as well-formed XML, keeping only the first root element and its
/// elements and attributes (text, comments, declarations, DOCTYPE are dropped):
///
/// * anything before the first element and after the root's end tag is skipped;
/// * a bare `&`, a `<` in an attribute value is escaped, control characters are dropped;
/// * a duplicate attribute keeps its first value, unquoted / value-less attributes are tolerated;
/// * an end tag closes the nearest open element of that name (closing the ones left open in
///   between), a stray end tag is ignored, elements still open at the end are closed;
/// * a `<Material>` directly inside a `<Material>` closes the outer one: a material never nests
///   directly (sub-materials sit in `<SubMaterials>`), so the outer one lost its end tag.
///
/// `None` when the text has no element.
fn repair_xml(text: &str) -> Option<String> {
    let b = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut stack: Vec<String> = Vec::new();
    let mut have_root = false;
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        let rest = &text[i..];
        if rest.starts_with("<!--") {
            i = after(text, i + 4, "-->");
        } else if rest.starts_with("<![CDATA[") {
            i = after(text, i + 9, "]]>");
        } else if rest.starts_with("<?") {
            i = after(text, i + 2, "?>");
        } else if rest.starts_with("<!") {
            i = after(text, i + 2, ">");
        } else if rest.starts_with("</") {
            let close = rest.find('>').map_or(text.len(), |p| i + p);
            let name = text[i + 2..close].trim();
            if let Some(pos) = stack.iter().rposition(|n| n.eq_ignore_ascii_case(name)) {
                while stack.len() > pos {
                    close_element(&mut out, &mut stack);
                }
                if stack.is_empty() {
                    break;
                }
            }
            i = close + 1;
        } else {
            let start = i;
            let Some((name, attrs, self_closing, end)) = start_tag(text, i + 1) else {
                i = start + 1; // a stray `<`
                continue;
            };
            i = end;
            if is_material(&name) && stack.last().is_some_and(|n| is_material(n)) {
                close_element(&mut out, &mut stack);
            }
            out.push('<');
            out.push_str(&name);
            for (k, v) in &attrs {
                out.push_str(&format!(" {k}=\"{v}\""));
            }
            have_root = true;
            if self_closing || stack.len() >= MAX_NEST {
                out.push_str("/>");
                if stack.is_empty() {
                    break;
                }
            } else {
                out.push('>');
                stack.push(name);
            }
        }
    }
    while !stack.is_empty() {
        close_element(&mut out, &mut stack);
    }
    have_root.then_some(out)
}

fn is_material(name: &str) -> bool {
    name.eq_ignore_ascii_case("Material")
}

fn close_element(out: &mut String, stack: &mut Vec<String>) {
    if let Some(name) = stack.pop() {
        out.push_str("</");
        out.push_str(&name);
        out.push('>');
    }
}

/// Index just past the next `pat` at or after `from` (or the end of the text).
fn after(text: &str, from: usize, pat: &str) -> usize {
    text[from..].find(pat).map_or(text.len(), |p| from + p + pat.len())
}

fn is_name_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.' | b':')
}

/// Name made of name bytes that starts with a letter or `_`; `:` (namespace) becomes `_`.
fn xml_name(s: &str) -> Option<String> {
    let first = *s.as_bytes().first()?;
    if !(first.is_ascii_alphabetic() || first == b'_') || !s.bytes().all(is_name_byte) {
        return None;
    }
    Some(s.replace(':', "_"))
}

/// Parses a start tag whose name begins at `at` (just after the `<`): `(name, attributes with
/// escaped values, self-closing, index after the tag)`. `None` when `<` does not begin a tag.
fn start_tag(text: &str, at: usize) -> Option<(String, Vec<(String, String)>, bool, usize)> {
    let b = text.as_bytes();
    let mut j = at;
    while j < b.len() && is_name_byte(b[j]) {
        j += 1;
    }
    let name = xml_name(&text[at..j])?;
    let mut attrs: Vec<(String, String)> = Vec::new();
    let mut self_closing = false;
    loop {
        while j < b.len() && b[j].is_ascii_whitespace() {
            j += 1;
        }
        if j >= b.len() {
            break;
        }
        match b[j] {
            b'>' => {
                j += 1;
                break;
            }
            b'/' if b.get(j + 1) == Some(&b'>') => {
                self_closing = true;
                j += 2;
                break;
            }
            b'/' => j += 1,
            b'<' => break, // the tag was never closed
            _ => {
                let key_at = j;
                while j < b.len() && !b" \t\r\n=/<>\"'".contains(&b[j]) {
                    j += 1;
                }
                if j == key_at {
                    j += 1; // a stray quote or `=`
                    continue;
                }
                let key = xml_name(&text[key_at..j]);
                while j < b.len() && b[j].is_ascii_whitespace() {
                    j += 1;
                }
                if b.get(j) != Some(&b'=') {
                    continue; // a value-less attribute
                }
                j += 1;
                while j < b.len() && b[j].is_ascii_whitespace() {
                    j += 1;
                }
                let value = match b.get(j) {
                    Some(&q @ (b'"' | b'\'')) => {
                        let v_at = j + 1;
                        match b[v_at..].iter().position(|&c| c == q) {
                            Some(p) => {
                                j = v_at + p + 1;
                                &text[v_at..v_at + p]
                            }
                            None => {
                                // never closed: runs up to the end of the tag
                                let end = b[v_at..].iter().position(|&c| c == b'>').map_or(b.len(), |p| v_at + p);
                                j = end;
                                &text[v_at..end]
                            }
                        }
                    }
                    _ => {
                        let v_at = j;
                        let ends_value = |j: usize| {
                            b[j].is_ascii_whitespace() || b[j] == b'>' || (b[j] == b'/' && b.get(j + 1) == Some(&b'>'))
                        };
                        while j < b.len() && !ends_value(j) {
                            j += 1;
                        }
                        &text[v_at..j]
                    }
                };
                if let Some(key) = key.filter(|k| !attrs.iter().any(|(n, _)| n == k)) {
                    attrs.push((key, escape_attr(value)));
                }
            }
        }
    }
    Some((name, attrs, self_closing, j))
}

/// Attribute value for a double-quoted XML attribute: escapes `<`, `"` and every `&` that is not
/// a valid reference, drops characters XML forbids.
fn escape_attr(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    for (i, c) in v.char_indices() {
        match c {
            '&' if is_reference(&v[i..]) => out.push('&'),
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '"' => out.push_str("&quot;"),
            '\t' | '\n' | '\r' => out.push(c),
            c if (c as u32) < 0x20 || c == '\u{fffe}' || c == '\u{ffff}' => {}
            c => out.push(c),
        }
    }
    out
}

/// Whether `s` starts with a predefined entity or a numeric character reference to an XML char.
fn is_reference(s: &str) -> bool {
    let Some(end) = s.find(';') else { return false };
    let body = &s[1..end];
    if matches!(body, "amp" | "lt" | "gt" | "quot" | "apos") {
        return true;
    }
    let code = match body.strip_prefix("#x").or_else(|| body.strip_prefix("#X")) {
        Some(hex) => u32::from_str_radix(hex, 16).ok(),
        None => body.strip_prefix('#').and_then(|dec| dec.parse::<u32>().ok()),
    };
    // from_u32 rejects surrogates; the rest is the XML `Char` production.
    code.and_then(char::from_u32)
        .is_some_and(|c| matches!(c, '\t' | '\n' | '\r') || (c as u32 >= 0x20 && c != '\u{fffe}' && c != '\u{ffff}'))
}

// ---------------------------------------------------------------------------------------------
// Wavefront .mtl (three CryEdit exports such as `001_ndeco/basket.mtl`)
// ---------------------------------------------------------------------------------------------

/// A Wavefront `.mtl` (`newmtl`, `Kd`, `Ks`, `Ns`, `d`, `map_Kd`): one material, or a multi
/// material when there are several. `None` without a `newmtl` line.
fn parse_wavefront(text: &str) -> Option<Material> {
    let mut mats: Vec<Material> = Vec::new();
    for line in text.lines() {
        let mut words = line.split_whitespace();
        let Some(key) = words.next() else { continue };
        if key == "newmtl" {
            mats.push(Material { name: words.collect::<Vec<_>>().join(" "), ..Material::default() });
            continue;
        }
        let Some(m) = mats.last_mut() else { continue };
        let nums: Vec<f32> = words.clone().map_while(parse_f32).collect();
        match key {
            "Kd" if nums.len() >= 3 => m.diffuse = Vec3::new(nums[0], nums[1], nums[2]),
            "Ks" if nums.len() >= 3 => m.specular = Vec3::new(nums[0], nums[1], nums[2]),
            "Ns" if !nums.is_empty() => m.shininess = nums[0],
            "d" if !nums.is_empty() => m.opacity = nums[0],
            "map_Kd" => {
                if let Some(file) = words.last() {
                    let file = file.to_string();
                    m.textures.push(Texture {
                        map: TextureMap::Diffuse, file, amount: None, tex_mod: None, attributes: Vec::new(),
                    });
                }
            }
            _ => {}
        }
    }
    match mats.len() {
        0 => None,
        1 => mats.pop(),
        _ => Some(Material { flags: flags::MULTI_SUBMTL, sub_materials: mats, ..Material::default() }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `game/objects/characters/elf/female/costume/el_f_login001/el_f_ar_cloth001.mtl`
    const ELF_CLOTH: &str = r#"<Material MtlFlags="256" SpecularLevel="1">
 <SubMaterials>
  <Material AlphaTest="0.5" Diffuse="0.21568628,0.21568628,0.21568628" Emissive="0,0,0" GenMask="4100009c" MtlFlags="524416" Name="SubMtl1" Opacity="1" Shader="Illum" Shininess="5" Specular="1,1,1" SpecularLevel="2" SurfaceType="" sRGB="2">
   <Textures>
    <Texture File="objects/Characters/elf/female/costume/el_f_login001/el_f_ar_cloth001_df.dds" Map="Diffuse">
     <TexMod/>
    </Texture>
    <Texture File="objects/Characters/elf/female/costume/el_f_login001/el_f_ar_cloth001_sp.dds" Map="Specular">
     <TexMod/>
    </Texture>
    <Texture Amount="50" File="objects/Characters/elf/female/costume/el_f_login001/el_f_ar_cloth001_ddn.dds" Map="Normalmap">
     <TexMod/>
    </Texture>
    <Texture File="game/textures/cubemaps/beach_blur2.dds" Map="Environment">
     <TexMod/>
    </Texture>
   </Textures>
   <PublicParams AmbientMultiplier="1" DiffuseModifierInHDR="1" EnvMapAmount="2.5999999" FresnelBias="0.52499998" FresnelScale="1.12" ReflectAmount="15" SpecularModifierInHDR="1" VirtuaCLDiffuse="0.57999998" VirtualCLSpecular="1.51"/>
  </Material>
  <Material Diffuse="0.18431373,0.16862746,0.14901961" Emissive="0,0,0" GenMask="a002" MtlFlags="524416" Name="skin" Opacity="1" Shader="HumanSkin" Shininess="41.5" Specular="0.16078432,0.4784314,0.50588238" SpecularLevel="1" SurfaceType="" sRGB="2">
   <Textures>
    <Texture File="game/objects/characters/elf/female/nude/el_f_nude_df.dds" Map="Diffuse">
     <TexMod/>
    </Texture>
    <Texture File="game/objects/characters/elf/female/nude/el_f_nude_sp.dds" Map="Specular">
     <TexMod/>
    </Texture>
    <Texture File="game/objects/characters/elf/female/nude/el_f_nude_ddn.dds" Map="Normalmap">
     <TexMod/>
    </Texture>
    <Texture File="game/textures/cubemaps/lib_reflection_02.dds" Map="Environment">
     <TexMod/>
    </Texture>
    <Texture File="game/objects/characters/elf/female/nude/el_f_nude_sss.dds" Map="SubSurface">
     <TexMod/>
    </Texture>
   </Textures>
   <PublicParams BSkinColor="0.51764709,0.61176473,0.70980394" DiffusionAmount="1.4" EnvFresnelBias="0" EnvFresnelScale="1" EnvReflectionAmount="0.25" MSkinColor="0.24313727,0.34509805,0.38431376" Melanin="0.75199997" RimPow="0.5" RimStrenght="0.02" SSSMapSat="0.40000001" SpecularStrenght="1" SubsurfaceStrenght="1" VirtuaCLDiffuse="1.5700001" VirtualCLSpecular="0"/>
  </Material>
  <Material Diffuse="1,1,1" Emissive="0,0,0" GenMask="0" MtlFlags="128" Name="cut" Opacity="1" Shader="Cut" Shininess="10" Specular="0,0,0" SpecularLevel="1" SurfaceType="">
   <Textures/>
  </Material>
 </SubMaterials>
</Material>"#;

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn multi_material() {
        let (m, route) = parse_tracked(ELF_CLOTH).unwrap();
        assert_eq!(route, Route::Strict);
        assert_eq!(m.flags, flags::MULTI_SUBMTL);
        assert_eq!(m.sub_materials.len(), 3);
        assert!(m.textures.is_empty());

        let cloth = m.sub(0).unwrap();
        assert_eq!(cloth.name, "SubMtl1");
        assert_eq!(cloth.shader, "Illum");
        assert_eq!(cloth.flags, 524416);
        assert_eq!(cloth.gen_mask, 0x4100_009c);
        assert!(near(cloth.alpha_test, 0.5) && near(cloth.shininess, 5.0) && near(cloth.specular_level, 2.0));
        assert!(near(cloth.diffuse.x, 0.215_686_28) && cloth.specular == Vec3::ONE && cloth.emissive == Vec3::ZERO);
        assert_eq!(cloth.textures.len(), 4);
        let normal = cloth.texture(&TextureMap::Normal).unwrap();
        assert_eq!(normal.amount, Some(50.0));
        assert_eq!(normal.tex_mod, None);
        assert_eq!(
            normal.pak_path(),
            "objects/characters/elf/female/costume/el_f_login001/el_f_ar_cloth001_ddn.dds"
        );
        assert_eq!(cloth.texture(&TextureMap::Diffuse).unwrap().amount, None);
        assert_eq!(cloth.texture(&TextureMap::Environment).unwrap().pak_path(), "textures/cubemaps/beach_blur2.dds");
        assert_eq!(cloth.public_param("reflectamount"), Some("15"));
        assert_eq!(cloth.public_params.len(), 9);
        assert_eq!(cloth.public_params[0], ("AmbientMultiplier".to_string(), "1".to_string()));

        let skin = m.sub(1).unwrap();
        assert_eq!((skin.name.as_str(), skin.shader.as_str(), skin.gen_mask), ("skin", "HumanSkin", 0xa002));
        assert_eq!(skin.textures.len(), 5);
        assert!(skin.texture(&TextureMap::SubSurface).is_some());
        assert!(near(skin.shininess, 41.5));

        let cut = m.sub(2).unwrap();
        assert_eq!((cut.shader.as_str(), cut.flags, cut.gen_mask), ("Cut", flags::PURE_CHILD, 0));
        assert!(cut.textures.is_empty());
        assert!(m.sub(3).is_none());
    }

    #[test]
    fn single_material_and_defaults() {
        let m = Material::parse(
            r#"<Material Name="rock" MtlFlags="2" Shader="Illum" GenMask="1000000" Opacity="0.5" GlowAmount="2.7" vertModifType="1" StringGenMask="%ALPHAGLOW">
                 <Textures><Texture Map="Diffuse" File="Objects\Rock_DF.TIF"/></Textures>
               </Material>"#,
        )
        .unwrap();
        assert!(m.sub_materials.is_empty());
        assert_eq!(m.sub(0), Some(&m));
        assert!(m.sub(1).is_none());
        assert!(m.two_sided() && !m.no_draw());
        assert!(near(m.opacity, 0.5) && near(m.glow_amount, 2.7) && m.vert_modif_type == 1);
        assert_eq!(m.string_gen_mask, "%ALPHAGLOW");
        assert_eq!(m.textures[0].pak_path(), "objects/rock_df.dds");
        // 64 bit flag missing: GenMask is decimal here.
        assert_eq!(m.gen_mask, 1_000_000);

        // Everything missing: the documented defaults.
        let d = Material::parse("<Material/>").unwrap();
        assert_eq!(d, Material::default());
        let d = Material::parse("<Material MtlFlags='256'><SubMaterials/></Material>").unwrap();
        assert_eq!(d.flags, 256);
        assert!(d.sub_materials.is_empty());
    }

    #[test]
    fn attribute_names_are_case_insensitive_but_values_are_kept() {
        let m = Material::parse(
            r#"<MATERIAL name="Hair" SHADER="Humanskin " mtlflags="524288" genmask="A002" SPECULARLEVEL="3" srgb="2" Alphatest="0.25" />"#,
        )
        .unwrap();
        assert_eq!(m.name, "Hair");
        assert_eq!(m.shader, "Humanskin");
        assert_eq!(m.gen_mask, 0xa002);
        assert!(near(m.specular_level, 3.0) && near(m.alpha_test, 0.25));
    }

    #[test]
    fn gen_mask_is_hex_with_the_64bit_flag_and_decimal_without() {
        assert_eq!(parse_gen_mask("4100009c", flags::SHADER_GEN_MASK_64BIT), 0x4100_009c);
        assert_eq!(parse_gen_mask("84000000094", flags::SHADER_GEN_MASK_64BIT), 0x8_4000_0000_94);
        assert_eq!(parse_gen_mask("ffffffffffffffff", flags::SHADER_GEN_MASK_64BIT), u64::MAX);
        assert_eq!(parse_gen_mask("0x10", flags::SHADER_GEN_MASK_64BIT), 0x10);
        assert_eq!(parse_gen_mask("16777216", 0), 0x100_0000);
        assert_eq!(parse_gen_mask("33284", 128), 0x8204);
        assert_eq!(parse_gen_mask("-1862270976", 128), 0x9100_0000);
        assert_eq!(parse_gen_mask("-2147483646", 0), 0x8000_0002);
        assert_eq!(parse_gen_mask("zz", 0), 0);
        // GenMask before MtlFlags in the attribute order.
        let m = Material::parse(r#"<Material GenMask="a002" MtlFlags="524416"/>"#).unwrap();
        assert_eq!(m.gen_mask, 0xa002);
    }

    #[test]
    fn colours() {
        assert_eq!(parse_color("0.5,0.25,1"), Some(Vec3::new(0.5, 0.25, 1.0)));
        assert_eq!(parse_color(" 1 , 2 , 3 "), Some(Vec3::new(1.0, 2.0, 3.0)));
        assert_eq!(parse_color("1 2 3"), Some(Vec3::new(1.0, 2.0, 3.0)));
        assert_eq!(parse_color("0.1,0.2,0.3,1"), Some(Vec3::new(0.1, 0.2, 0.3)));
        assert_eq!(parse_color("1,2"), None);
        assert_eq!(parse_color("a,b,c"), None);
        assert_eq!(parse_color(""), None);
        assert_eq!(parse_color("nan,0,0"), None);
        let m = Material::parse(r#"<Material Diffuse="oops" Specular="0.1,0.2" Emissive="1,0.5,0.25"/>"#).unwrap();
        assert_eq!((m.diffuse, m.specular), (Vec3::ONE, Vec3::ZERO));
        assert_eq!(m.emissive, Vec3::new(1.0, 0.5, 0.25));
    }

    #[test]
    fn texture_maps() {
        let m = Material::parse(
            r#"<Material><Textures>
                 <Texture Map="Diffuse" File="a_df.dds"/>
                 <Texture Map="specular" File="a_sp.dds"/>
                 <Texture Map="Normalmap" File="a_ddn.dds"/>
                 <Texture Map="Bumpmap" File="a_bump.dds"/>
                 <Texture Map="Environment" File="e.dds"/>
                 <Texture Map="SubSurface" File="s.dds"/>
                 <Texture Map="Opacity" File="o.dds"/>
                 <Texture Map="Detail" File="d.dds"/>
                 <Texture Map="Decal" File="dc.dds"/>
                 <Texture Map="Custom" File="c.dds"/>
                 <Texture Map="[1] Custom" File="c1.dds"/>
                 <Texture Map="Heightmap" File="h.dds"/>
                 <Texture Map="Diffuse" File=""/>
                 <Texture Map="Diffuse"/>
               </Textures></Material>"#,
        )
        .unwrap();
        let maps: Vec<_> = m.textures.iter().map(|t| t.map.clone()).collect();
        use TextureMap::*;
        let want = [Diffuse, Specular, Normal, Bump, Environment, SubSurface, Opacity, Detail, Decal, Custom, Custom1, Other("Heightmap".into())];
        assert_eq!(maps, want, "textures without a file are dropped");
        assert!(m.texture(&Custom1).is_some() && m.texture(&Custom).unwrap().file == "c.dds");
        assert_eq!(m.textures[0], Texture {
            map: Diffuse, file: "a_df.dds".into(), amount: None, tex_mod: None, attributes: Vec::new(),
        });
        assert!(m.textures.iter().all(|t| t.attributes.is_empty()));
    }

    #[test]
    fn texture_attributes_keep_raw_spelling_values_and_presence() {
        let m = Material::parse(
            r#"<Material><Textures>
                 <Texture Map="Opacity" File="absent.dds"/>
                 <Texture Map="Opacity" File="empty.dds" IsTileU="" IsTileV="" Filter="" TexType=""/>
                 <Texture mAp="Diffuse" fIlE="raw.dds" aMoUnT="50" iSTileU="0" istilev="true"
                          fIlTeR="not-an-integer" tExTyPe="  +3junk " FutureFlag=" keep &amp; text ">
                   <TexMod TileU="2" ChildOnly="child"/>
                 </Texture>
               </Textures></Material>"#,
        ).unwrap();
        assert!(m.textures[0].attributes.is_empty());
        let empty: Vec<_> = m.textures[1].attributes.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        assert_eq!(empty, [("IsTileU", ""), ("IsTileV", ""), ("Filter", ""), ("TexType", "")]);
        let raw = &m.textures[2];
        assert_eq!((raw.map.clone(), raw.file.as_str(), raw.amount), (TextureMap::Diffuse, "raw.dds", Some(50.0)));
        let attributes: Vec<_> = raw.attributes.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        assert_eq!(attributes, [
            ("iSTileU", "0"), ("istilev", "true"), ("fIlTeR", "not-an-integer"),
            ("tExTyPe", "  +3junk "), ("FutureFlag", " keep & text "),
        ]);
        assert_eq!(raw.tex_mod.as_ref().unwrap().tile_u, 2.0);
        assert_eq!(m.clone().textures, m.textures, "cloning retains raw authoring alongside parsed texture fields");
    }

    #[test]
    fn tex_mod_values() {
        let m = Material::parse(
            r#"<Material><Textures>
                 <Texture Map="Diffuse" File="a.dds"><TexMod TileU="0.5" TileV="2" OffsetU="0.25" OffsetV="-0.1" RotateW="90" TexMod_UOscillatorType="2"/></Texture>
                 <Texture Map="Specular" File="b.dds"><TexMod TileU="1" TileV="1" OffsetU="0" RotateW="0" TexMod_UOscillatorType="2"/></Texture>
                 <Texture Map="Normalmap" File="c.dds"><TexMod TileV="4"/></Texture>
                 <Texture Map="Environment" File="d.dds"><TexMod/></Texture>
                 <Texture Map="Detail" File="e.dds"/>
                 <Texture Map="Decal" File="f.dds"><TexMod tileu="bad" RotateU="45" OffsetV="0.5"/></Texture>
               </Textures></Material>"#,
        )
        .unwrap();
        let t = &m.textures;
        assert_eq!(
            t[0].tex_mod,
            Some(TexMod {
                tile_u: 0.5,
                tile_v: 2.0,
                offset_u: 0.25,
                offset_v: -0.1,
                rotate_w: 90.0,
                unhandled: vec![("TexMod_UOscillatorType".into(), "2".into())]
            })
        );
        assert_eq!(
            t[1].tex_mod,
            Some(TexMod {
                unhandled: vec![("TexMod_UOscillatorType".into(), "2".into())],
                ..TexMod::default()
            })
        );
        assert_eq!(
            t[2].tex_mod,
            Some(TexMod {
                tile_v: 4.0,
                ..TexMod::default()
            })
        );
        assert_eq!(t[3].tex_mod, None);
        assert_eq!(t[4].tex_mod, None);
        assert_eq!(
            t[5].tex_mod,
            Some(TexMod {
                offset_v: 0.5,
                unhandled: vec![("RotateU".into(), "45".into())],
                ..TexMod::default()
            })
        );
    }

    #[test]
    fn sub_materials_keep_their_indices() {
        let m = Material::parse(
            r#"<Material MtlFlags="256"><SubMaterials>
                 <MaterialRef />
                 <Material Name="a" Shader="Illum"/>
                 <MaterialRef Name="library_mat"/>
                 <Material Name="c" MtlFlags="256"><SubMaterials><Material Name="c0"/></SubMaterials></Material>
               </SubMaterials></Material>"#,
        )
        .unwrap();
        assert_eq!(m.sub_materials.len(), 4);
        assert_eq!(m.sub(0), Some(&Material::default()));
        assert_eq!(m.sub(1).unwrap().shader, "Illum");
        assert_eq!(m.sub(2).unwrap().name, "library_mat");
        assert_eq!(m.sub(3).unwrap().sub(0).unwrap().name, "c0");
        assert!(m.sub(4).is_none());
    }

    #[test]
    fn bom_and_leading_junk() {
        for text in [
            "\u{feff}<Material Name=\"a\"/>",
            "\r\n<Material Name=\"a\"/>",
            "\u{feff}\r\n<?xml version=\"1.0\" encoding=\"utf-8\"?>\r\n<!-- exported -->\r\n<Material Name=\"a\"/>\r\n",
            "# Exported by something\r\n<Material Name=\"a\"/>",
            "  junk <b> junk\n<Material Name=\"a\"/>",
            "<Material Name=\"a\"/>\r\n<!-- trailing -->\r\n",
        ] {
            assert_eq!(Material::parse(text).unwrap().name, "a", "{text:?}");
        }
        assert!(Material::parse("").is_err());
        assert!(Material::parse("   \r\n").is_err());
        assert!(Material::parse("# Material file\r\nnot a material").is_err());
        assert!(Material::parse("<Sky><Cloud/></Sky>").is_err());
    }

    #[test]
    fn sloppy_xml_is_repaired() {
        let route = |text: &str| parse_tracked(text).unwrap();
        // A bare `&` (and an unknown entity), `<` inside a value.
        let (m, r) = route(r#"<Material Name="rock & roll &nbsp; a<b &amp; &#65; &#x42; &#0;"/>"#);
        assert!(matches!(r, Route::Repaired(_)));
        assert_eq!(m.name, "rock & roll &nbsp; a<b & A B &#0;");
        // A duplicate attribute keeps the first.
        let (m, r) = route(r#"<Material Name="first" Shader="Illum" Name="second"/>"#);
        assert!(matches!(r, Route::Repaired(_)));
        assert_eq!((m.name.as_str(), m.shader.as_str()), ("first", "Illum"));
        // Junk after the root.
        let (m, r) = route("<Material Name=\"a\"/>\r\n<Material Name=\"b\"/>\r\nbla bla <<");
        assert!(matches!(r, Route::Repaired(_)));
        assert_eq!(m.name, "a");
        let (m, _) = route("<Material Name=\"a\"><Textures/></Material>\r\n</Material></x>garbage");
        assert_eq!(m.name, "a");
        // Control characters.
        let (m, r) = route("<Material Name=\"a\u{1}b\u{8}c\"/>\u{0}");
        assert!(matches!(r, Route::Repaired(_)));
        assert_eq!(m.name, "abc");
        // Bytes that were not UTF-8 (`Material::load` goes through `from_utf8_lossy`).
        let lossy = String::from_utf8_lossy(b"<Material Name=\"caf\xe9\"/>").into_owned();
        assert_eq!(Material::parse(&lossy).unwrap().name, "caf\u{fffd}");
        // Unquoted / value-less / single quoted attributes, an unterminated tag, a truncated file.
        let (m, _) = route("<Material Name=abc Shader='Illum' flag MtlFlags=\"2\"><Textures><Texture Map=\"Diffuse\" File=\"a.dds\"");
        assert_eq!((m.name.as_str(), m.shader.as_str(), m.flags), ("abc", "Illum", 2));
        assert_eq!(m.textures.len(), 1);
        // Mismatched end tags.
        let (m, _) = route("<Material><Textures><Texture Map=\"Diffuse\" File=\"a.dds\"></Textures></Foo><PublicParams a=\"1\"/></Material>");
        assert_eq!(m.textures.len(), 1);
        assert_eq!(m.public_param("a"), Some("1"));
    }

    /// `game/materials/aa_terrain/canyon/tr_wt_dirt3d_300a.mtl`: the first sub-material has no end
    /// tag, so the rest nest inside it.
    #[test]
    fn unclosed_sub_material() {
        let (m, r) = parse_tracked(
            r#"<Material MtlFlags="256" SpecularLevel="1">
 <SubMaterials>
  <Material Name="x" MtlFlags="172" Shader="Terrain.Layer">
  <Material Name="x" MtlFlags="172" Shader="Terrain.Layer">
   <Textures>
    <Texture Map="Diffuse" File="textures/aa_terrain/wasteland/tr_wt_dirt_300a_df.dds" Filter="5">
     <TexMod TileU="0.039999999" TileV="0.039999999"/>
    </Texture>
   </Textures>
   <PublicParams DetailTextureStrength="1.473"/>
  </Material>
  <Material Name="y" MtlFlags="172" Shader="Terrain.Layer">
   <PublicParams DetailTextureStrength="1"/>
  </Material>
 </SubMaterials>
 <PublicParams AmbientMultiplier="1"/>
</Material>"#,
        )
        .unwrap();
        assert!(matches!(r, Route::Repaired(_)));
        let names: Vec<_> = m.sub_materials.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["x", "x", "y"]);
        assert!(m.sub_materials[0].textures.is_empty());
        assert_eq!(m.sub_materials[1].textures.len(), 1);
        assert_eq!(m.sub_materials[1].textures[0].tex_mod.as_ref().unwrap().tile_u, 0.039_999_999);
        assert_eq!(m.public_param("AmbientMultiplier"), Some("1"));
    }

    /// `game/objects/env/01_nuia/001_ndeco/basket.mtl` and two more are Wavefront files.
    #[test]
    fn wavefront_mtl() {
        let text = "# Material file exported by CryEdit\r\n\r\nnewmtl Terrain\r\nKa 1.000000 1.000000 1.000000\r\nKd 0.5 0.25 1.000000\r\nKs 1.000000 1.000000 1.000000\r\nNs 0.000000\r\nmap_Kd Terrain.bmp";
        let (m, r) = parse_tracked(text).unwrap();
        assert_eq!(r, Route::Wavefront);
        assert_eq!(m.name, "Terrain");
        assert_eq!(m.diffuse, Vec3::new(0.5, 0.25, 1.0));
        assert_eq!(m.shininess, 0.0);
        assert_eq!(m.textures.len(), 1);
        assert_eq!(m.textures[0].pak_path(), "terrain.dds");
        assert!(m.textures[0].attributes.is_empty());
        let m = Material::parse("newmtl a\nKd 1 0 0\nnewmtl b\n").unwrap();
        assert_eq!(m.sub_materials.len(), 2);
        assert_eq!(m.sub(0).unwrap().diffuse, Vec3::new(1.0, 0.0, 0.0));
    }

    /// Whatever the text, parsing returns (no panic on odd UTF-8 boundaries, no endless loop).
    #[test]
    fn mangled_text_never_panics() {
        let text = ELF_CLOTH.replace("Name=\"skin\"", "Name=\"sk\u{e9}\u{4e2d}n\"").replace("</Textures>", "\u{e9}</Textures>");
        let mut parsed = 0;
        for (cut, _) in text.char_indices() {
            parsed += Material::parse(&text[..cut]).is_ok() as usize;
            // Cut a hole instead of the tail.
            let hole = text[cut..].chars().next().map_or(cut, |c| cut + c.len_utf8());
            let holed = format!("{}{}", &text[..cut], &text[hole..]);
            parsed += Material::parse(&holed).is_ok() as usize;
        }
        assert!(parsed > text.len(), "most mangled texts still give a material ({parsed})");
        for junk in ["<", "<<<", "</", "<M", "<M a", "<M a=", "<M a=\"", "<M a='\u{e9}", "<M a=\u{e9}\u{e9} b", "<M/", "<M/>/", "&", "<!--", "<![CDATA[", "<?"] {
            let _ = Material::parse(junk);
            let _ = Material::parse(&format!("<Material>{junk}"));
            let _ = Material::parse(&format!("<Material Name=\"a\" {junk}"));
        }
    }

    #[test]
    fn deep_nesting_is_cut_off() {
        let n = 200;
        let text = format!("{}<Material Name=\"leaf\"/>{}", "<Material><SubMaterials>".repeat(n), "</SubMaterials></Material>".repeat(n));
        let mut m = &Material::parse(&text).unwrap();
        let mut depth = 0;
        while let Some(s) = m.sub_materials.first() {
            m = s;
            depth += 1;
        }
        assert_eq!(depth, MAX_DEPTH);
    }

    #[test]
    fn texture_paths() {
        let p = texture_pak_path;
        assert_eq!(p(""), "");
        assert_eq!(p("   "), "");
        assert_eq!(p("game/textures/aa_terrain/decals/"), "");
        assert_eq!(p("game/"), "");
        assert_eq!(p("Game\\Objects\\Env\\Rock_DF.DDS"), "objects/env/rock_df.dds");
        assert_eq!(p("objects//env///rock.dds"), "objects/env/rock.dds");
        assert_eq!(p("./objects/env/rock.dds"), "objects/env/rock.dds");
        assert_eq!(p("/objects/env/rock.dds"), "objects/env/rock.dds");
        assert_eq!(p(".\\game\\objects\\rock.dds"), "objects/rock.dds");
        assert_eq!(p("game/game/objects/rock.dds"), "game/objects/rock.dds");
        assert_eq!(p("textures/p_effects/lightnig_flash_02.tif"), "textures/p_effects/lightnig_flash_02.dds");
        assert_eq!(p("Game/textures/a.TIF"), "textures/a.dds");
        assert_eq!(p("textures/a_ddn.tga"), "textures/a_ddn.dds");
        assert_eq!(p("textures/a.png"), "textures/a.dds");
        assert_eq!(p("textures/a_df.dd"), "textures/a_df.dds");
        assert_eq!(p("old/barrel_oak_b_df.ddn"), "old/barrel_oak_b_df.dds");
        assert_eq!(p("housing/hs_f_drums_ddn"), "housing/hs_f_drums_ddn.dds");
        assert_eq!(p("a.b/hs_f_drums_ddn"), "a.b/hs_f_drums_ddn.dds");
        assert_eq!(p("textures/a.b.tif"), "textures/a.b.dds");
        assert_eq!(p("sounds/a.fsb"), "sounds/a.fsb", "only image extensions are mapped");
        assert_eq!(p(" textures/a.dds "), "textures/a.dds");
    }

    // -- the real pak -------------------------------------------------------------------------

    fn mtl_names() -> Vec<&'static str> {
        let mut v: Vec<&str> = aa_pak::Pak::global().names().filter(|n| n.to_ascii_lowercase().ends_with(".mtl")).collect();
        v.sort_unstable();
        v
    }

    fn load_text(name: &str) -> Result<String> {
        Ok(String::from_utf8_lossy(&crate::read(name)?).into_owned())
    }

    #[test]
    fn parses_sample_of_pak_mtl() {
        if !crate::pak_available() {
            eprintln!("skipped: no game_pak");
            return;
        }
        let names = mtl_names();
        assert!(names.len() > 30_000, "{} .mtl files", names.len());
        let mut failures = Vec::new();
        let mut parsed = 0;
        for name in names.iter().step_by(37) {
            match load_text(name).and_then(|t| Material::parse(&t)) {
                Ok(_) => parsed += 1,
                Err(e) => failures.push(format!("{name}: {e}")),
            }
        }
        assert!(failures.is_empty(), "{} of {} failed:\n{}", failures.len(), parsed + failures.len(), failures.join("\n"));
        assert!(parsed > 900, "{parsed}");
        // The sample must include the multi-material login costume.
        let cloth = Material::load("game/objects/characters/elf/female/costume/el_f_login001/el_f_ar_cloth001.mtl").unwrap();
        assert_eq!(cloth.sub_materials.len(), 3);
        assert_eq!(cloth.sub(1).unwrap().shader, "HumanSkin");
    }

    /// Parses every `.mtl` of the pak and prints statistics:
    /// `cargo test -p aa-cry parses_every_mtl -- --ignored --nocapture`
    #[test]
    #[ignore = "parses all 38k materials; prints statistics"]
    fn parses_every_mtl_in_pak() {
        use std::collections::{BTreeMap, HashMap, HashSet};
        if !crate::pak_available() {
            eprintln!("skipped: no game_pak");
            return;
        }
        let pak = aa_pak::Pak::global();
        let names = mtl_names();
        let dds_names: HashSet<String> = pak
            .names()
            .filter(|n| n.to_ascii_lowercase().ends_with(".dds"))
            .filter_map(|n| n.rsplit('/').next())
            .map(str::to_ascii_lowercase)
            .collect();

        let started = std::time::Instant::now();
        let mut failures: Vec<(String, String)> = Vec::new();
        let mut routes: BTreeMap<String, (usize, &str)> = BTreeMap::new();
        let mut shaders: HashMap<String, usize> = HashMap::new();
        let mut other_maps: HashMap<String, usize> = HashMap::new();
        let (mut files, mut materials, mut multi, mut textures, mut with_tex_mod) = (0, 0, 0, 0, 0);
        let (mut raw_ok, mut clean_ok, mut mapped_ok) = (0, 0, 0);
        let mut unresolved: BTreeMap<&str, (usize, Vec<String>)> = BTreeMap::new();
        let mut unresolved_dirs: HashMap<String, usize> = HashMap::new();
        let mut regressions: Vec<String> = Vec::new(); // resolve as written, but not after the mapping

        fn walk<'a>(m: &'a Material, f: &mut impl FnMut(&'a Material)) {
            f(m);
            m.sub_materials.iter().for_each(|s| walk(s, f));
        }

        for name in &names {
            let parsed = load_text(name).and_then(|t| parse_tracked(&t));
            let (root, route) = match parsed {
                Ok(ok) => ok,
                Err(e) => {
                    failures.push((name.to_string(), e.to_string()));
                    continue;
                }
            };
            files += 1;
            let key = match route {
                Route::Strict => "strict xml".to_string(),
                Route::Wavefront => "wavefront".to_string(),
                Route::Repaired(why) => format!("repaired: {}", why.split(" at ").next().unwrap_or(&why)),
            };
            let entry = routes.entry(key).or_insert((0, *name));
            entry.0 += 1;
            if !root.sub_materials.is_empty() {
                multi += 1;
            }
            walk(&root, &mut |m| {
                materials += 1;
                *shaders.entry(m.shader.clone()).or_default() += 1;
                for t in &m.textures {
                    textures += 1;
                    with_tex_mod += t.tex_mod.is_some() as usize;
                    if let TextureMap::Other(o) = &t.map {
                        *other_maps.entry(o.clone()).or_default() += 1;
                    }
                    let mapped = t.pak_path();
                    raw_ok += pak.exists(&t.file) as usize;
                    let cleaned = pak.exists(&clean_path(&t.file));
                    clean_ok += cleaned as usize;
                    if pak.exists(&mapped) {
                        mapped_ok += 1;
                        continue;
                    }
                    if cleaned {
                        regressions.push(format!("{} (in {name})", t.file));
                    }
                    let base = mapped.rsplit('/').next().unwrap_or("");
                    let ext = t.file.rsplit_once('.').map_or("", |(_, e)| e).to_ascii_lowercase();
                    let cause = if mapped.is_empty() {
                        "File names a directory"
                    } else if ext != "dds" && SOURCE_IMAGE_EXTS.contains(&ext.as_str()) {
                        "source image (.tif/.tga/...) and no .dds of that name"
                    } else if dds_names.contains(base) {
                        ".dds missing here, same file name elsewhere in the pak (moved)"
                    } else {
                        ".dds missing, no file of that name in the pak"
                    };
                    let (count, examples) = unresolved.entry(cause).or_default();
                    *count += 1;
                    if examples.len() < 3 {
                        examples.push(format!("{} (in {name})", t.file));
                    }
                    if !mapped.is_empty() {
                        let dir: Vec<&str> = mapped.split('/').take(3).collect();
                        *unresolved_dirs.entry(dir.join("/")).or_default() += 1;
                    }
                }
            });
        }

        let top = |m: &HashMap<String, usize>, n: usize| {
            let mut v: Vec<_> = m.iter().collect();
            v.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
            v.into_iter().take(n).map(|(k, c)| format!("{k:?}={c}")).collect::<Vec<_>>().join(", ")
        };
        let pct = |n: usize| 100.0 * n as f64 / textures.max(1) as f64;
        eprintln!("--- {} .mtl files in {:.1?}", names.len(), started.elapsed());
        eprintln!("parsed {files}, FAILED {}, multi-material roots {multi}, materials {materials}", failures.len());
        for (k, (n, example)) in &routes {
            eprintln!("  {n:6} {k}   e.g. {example}");
        }
        for (name, e) in failures.iter().take(20) {
            eprintln!("  FAIL {name}: {e}");
        }
        eprintln!("shaders ({} distinct): {}", shaders.len(), top(&shaders, 60));
        eprintln!("other texture maps: {}", top(&other_maps, 20));
        eprintln!("textures {textures} ({with_tex_mod} with a non-default TexMod)");
        eprintln!("  resolve as written (lowercase, game/ prefix):      {raw_ok:7} ({:.2}%)", pct(raw_ok));
        eprintln!("  resolve after path cleanup, no extension mapping:  {clean_ok:7} ({:.2}%)", pct(clean_ok));
        eprintln!("  resolve with texture_pak_path (cleanup + .dds):    {mapped_ok:7} ({:.2}%)", pct(mapped_ok));
        for (cause, (count, examples)) in &unresolved {
            eprintln!("  unresolved {count:5}: {cause}  e.g. {}", examples.join("; "));
        }
        eprintln!("  unresolved texture dirs: {}", top(&unresolved_dirs, 12));
        eprintln!("  resolved before the mapping, lost by it: {}  e.g. {}", regressions.len(), regressions.first().map_or("", |s| s));
        assert!(mapped_ok > clean_ok + regressions.len(), "the .dds mapping must pay off");
        assert!(failures.is_empty(), "{} .mtl files failed to parse", failures.len());
        assert_eq!(files, names.len());
    }
}
