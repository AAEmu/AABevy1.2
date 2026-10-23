//! The sea's own inputs: the world's `env.xml` `<Ocean Material CausticDepth CausticIntensity CausticsTilling>` and
//! `<OceanAnimation WindDirection WindSpeed>`, and the public foam parameters of that ocean material
//! (`materials/ocean/ocean.mtl`, shader `Water`, `water.cfx`). See docs/aa-world.md "Native ocean foam and caustics".

/// `water.ext` FOAM (the mask is hexadecimal when MtlFlags has 0x80000, see aa-cry).
const FOAM: u64 = 0x400;

/// What the sea shader needs from the original data (defaults: the `water.cfx` / `deferredcaustics` defaults).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OceanParams {
    /// `FoamSoftIntersectionFactor`, `FoamCoverFactor`, `FoamCoverOffset`, `FoamCoverBlendWeight`.
    pub foam_soft: f32,
    pub foam_cover_factor: f32,
    pub foam_cover_offset: f32,
    pub foam_cover_blend: f32,
    /// `saturate(FoamAmount)`; zero when the material has no FOAM.
    pub foam_amount: f32,
    /// Bump `Tilling` and `FoamTilling`: the foam repeats `0.005 * Tilling * FoamTilling` times per metre.
    pub tilling: f32,
    pub foam_tilling: f32,
    /// `OceanAnimation`: wind direction (radians) and speed, drift of the bump/foam coordinates.
    pub wind_direction: f32,
    pub wind_speed: f32,
    /// `OceanAnimation` `WavesAmount`, `WavesSize` and `WavesSpeed`: the strength, the size (the swell repeats every
    /// `64 / WavesSize` metres, `WaterVS` `tcFFT`) and the speed of the vertex displacement.
    pub waves_amount: f32,
    pub waves_size: f32,
    pub waves_speed: f32,
    /// `Ocean CausticIntensity`, `CausticDepth` (metres) and `CausticsTilling`.
    pub caustic_intensity: f32,
    pub caustic_depth: f32,
    pub caustic_tilling: f32,
}

impl Default for OceanParams {
    fn default() -> Self {
        OceanParams {
            foam_soft: 0.75,
            foam_cover_factor: 0.0,
            foam_cover_offset: 0.15,
            foam_cover_blend: 1.1,
            foam_amount: 0.0,
            tilling: 10.0,
            foam_tilling: 12.0,
            wind_direction: 1.0,
            wind_speed: 4.0,
            waves_amount: 1.0,
            waves_size: 1.0,
            waves_speed: 1.0,
            caustic_intensity: 0.0,
            caustic_depth: 8.0,
            caustic_tilling: 1.0,
        }
    }
}

impl OceanParams {
    /// The sea of `world` (`None`: the world has no `env.xml` `<Ocean>`).
    pub fn load(world: &str) -> Option<OceanParams> {
        let env = aa_pak::Pak::global().read_string(&format!("worlds/{world}/env.xml")).ok()?;
        let path = ocean_material(&env)?;
        let mtl = aa_cry::read(&format!("{}.mtl", path.trim_start_matches("game/").trim_end_matches(".mtl")))
            .ok().map(|b| String::from_utf8_lossy(&b).into_owned());
        Some(OceanParams::parse(&env, mtl.as_deref()))
    }

    pub fn parse(env_xml: &str, mtl: Option<&str>) -> OceanParams {
        let mut o = OceanParams::default();
        if let Ok(doc) = roxmltree::Document::parse(env_xml) {
            let node = |tag: &str| doc.root_element().children().find(|n| n.has_tag_name(tag));
            let f = |n: Option<roxmltree::Node>, name: &str, default: f32| {
                n.and_then(|n| n.attribute(name)).and_then(|v| v.trim().parse().ok()).unwrap_or(default)
            };
            let ocean = node("Ocean");
            o.caustic_intensity = f(ocean, "CausticIntensity", o.caustic_intensity);
            o.caustic_depth = f(ocean, "CausticDepth", o.caustic_depth);
            o.caustic_tilling = f(ocean, "CausticsTilling", o.caustic_tilling);
            let animation = node("OceanAnimation");
            o.wind_direction = f(animation, "WindDirection", o.wind_direction);
            o.wind_speed = f(animation, "WindSpeed", o.wind_speed);
            o.waves_amount = f(animation, "WavesAmount", o.waves_amount);
            o.waves_size = f(animation, "WavesSize", o.waves_size).max(0.05);
            o.waves_speed = f(animation, "WavesSpeed", o.waves_speed);
        }
        let Some(mtl) = mtl.and_then(|t| aa_cry::material::Material::parse(t).ok()) else { return o };
        if !mtl.shader.eq_ignore_ascii_case("Water") {
            return o;
        }
        let p = |name: &str, default: f32| mtl.public_param(name).and_then(|v| v.parse().ok()).unwrap_or(default);
        o.tilling = p("Tilling", o.tilling);
        if mtl.gen_mask & FOAM != 0 {
            o.foam_soft = p("FoamSoftIntersectionFactor", o.foam_soft);
            o.foam_cover_factor = p("FoamCoverFactor", o.foam_cover_factor);
            o.foam_cover_offset = p("FoamCoverOffset", o.foam_cover_offset);
            o.foam_cover_blend = p("FoamCoverBlendWeight", o.foam_cover_blend);
            o.foam_tilling = p("FoamTilling", o.foam_tilling);
            o.foam_amount = p("FoamAmount", 1.0).clamp(0.0, 1.0);
        }
        o
    }

    /// Foam texture repeats per metre of world Cry XY (`WaterVS` `vPos.xy * 0.005 * Tilling`, then `* FoamTilling`).
    pub fn foam_scale(&self) -> f32 {
        0.005 * self.tilling * self.foam_tilling
    }

    /// Drift of the bump/foam coordinates per second, in foam texture units (`WaterVS`:
    /// `time * OceanParams0.y * 0.0025 * FlowDir`, times `FoamTilling`). Taking `OceanParams0.y` as
    /// `WindSpeed` and `FlowDir` as the wind direction's cosine/sine is the CryEngine 3 convention, not read from 1.2.
    pub fn foam_drift(&self) -> [f32; 2] {
        let s = self.wind_speed * 0.0025 * self.foam_tilling;
        [self.wind_direction.cos() * s, self.wind_direction.sin() * s]
    }
}

/// Peak height in metres of the swell at `WavesAmount` 1.6 (main_world): the engine adds the FFT texture to the grid vertices with
/// `0.06 x OceanParams1.x x (1.5, 1.5, 1)` (`water.cfx` `ApplyVertexDisplacement`; the texture itself is generated by the renderer
/// and not in the data), so the swell of the 1.2 sea is some decimetres.
pub const SWELL_PEAK: f32 = 0.35;

impl OceanParams {
    /// The swell's parameters for the shader: x peak height (m), y repeat length of the base wave (m), z speed factor,
    /// w wind direction (radians, Cry XY).
    pub fn swell(&self) -> [f32; 4] {
        [SWELL_PEAK * self.waves_amount / 1.6, 64.0 / self.waves_size, self.waves_speed, self.wind_direction]
    }
}

/// The sea's grid: a polar mesh around its centre (`r = 1.5 m` .. `radius`, 256 segments, rings 2.45 % apart, so the cells keep
/// the same shape from a few metres to the horizon and the swell has no T-junctions). Y up, normals up, UV = XZ in metres.
pub fn ocean_mesh(radius: f32) -> bevy::mesh::Mesh {
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::{Indices, Mesh, PrimitiveTopology};
    const SEGMENTS: usize = 256;
    let ratio = 1.0 + std::f32::consts::TAU / SEGMENTS as f32;
    let first = 1.5f32;
    let rings = ((radius / first).ln() / ratio.ln()).ceil() as usize + 1;
    let mut positions = Vec::with_capacity(1 + rings * SEGMENTS);
    positions.push([0.0, 0.0, 0.0]);
    let mut r = first;
    for _ in 0..rings {
        for s in 0..SEGMENTS {
            let a = s as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
            positions.push([r * a.cos(), 0.0, r * a.sin()]);
        }
        r *= ratio;
    }
    let ring = |i: usize, s: usize| (1 + i * SEGMENTS + s % SEGMENTS) as u32;
    let mut indices = Vec::with_capacity((SEGMENTS + (rings - 1) * SEGMENTS * 2) * 3);
    for s in 0..SEGMENTS {
        // counter clockwise seen from above (+Y): the angle runs from +X towards +Z, i.e. clockwise from above
        indices.extend([0, ring(0, s + 1), ring(0, s)]);
    }
    for i in 0..rings - 1 {
        for s in 0..SEGMENTS {
            let (a, b, c, d) = (ring(i, s), ring(i, s + 1), ring(i + 1, s), ring(i + 1, s + 1));
            indices.extend([a, b, c, b, d, c]);
        }
    }
    let uv: Vec<[f32; 2]> = positions.iter().map(|p| [p[0], p[2]]).collect();
    let normals = vec![[0.0, 1.0, 0.0]; positions.len()];
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// `<Ocean Material="...">` of a world `env.xml`.
fn ocean_material(env_xml: &str) -> Option<String> {
    let doc = roxmltree::Document::parse(env_xml).ok()?;
    let node = doc.root_element().children().find(|n| n.has_tag_name("Ocean"))?;
    node.attribute("Material").map(str::trim).filter(|m| !m.is_empty()).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_world_ocean_foam_and_caustics() {
        let o = OceanParams::load("main_world").expect("main_world env.xml has an Ocean");
        // env.xml: <Ocean Material="game/materials/ocean/ocean" CausticDepth="8" CausticIntensity="0.03" CausticsTilling="1"/>
        assert_eq!((o.caustic_depth, o.caustic_intensity, o.caustic_tilling), (8.0, 0.03, 1.0));
        assert_eq!((o.wind_direction, o.wind_speed), (1.0, 4.0));
        // ocean.mtl: MtlFlags 0x80800 -> GenMask "421" is hex: BUMP_MAP | SUN_SHINE | FOAM
        assert_eq!(o.foam_amount, 1.0);
        assert!((o.foam_soft - 0.7).abs() < 1e-6 && (o.foam_cover_factor + 0.045).abs() < 1e-6);
        assert!((o.foam_cover_offset - 0.079).abs() < 1e-6 && (o.foam_cover_blend - 1.1).abs() < 1e-6);
        assert_eq!((o.tilling, o.foam_tilling), (12.0, 8.0));
        assert!((o.foam_scale() - 0.48).abs() < 1e-6);
        // env.xml: <OceanAnimation WindDirection="1" WindSpeed="4" WavesSpeed="1" WavesAmount="1.6" WavesSize="0.75"/>
        assert_eq!((o.waves_amount, o.waves_size, o.waves_speed), (1.6, 0.75, 1.0));
        let s = o.swell();
        assert!((s[0] - SWELL_PEAK).abs() < 1e-6 && (s[1] - 64.0 / 0.75).abs() < 1e-3);
    }

    #[test]
    fn ocean_mesh_covers_the_radius_and_faces_up() {
        let mesh = ocean_mesh(9000.0);
        let bevy::mesh::VertexAttributeValues::Float32x3(p) = mesh.attribute(bevy::mesh::Mesh::ATTRIBUTE_POSITION).unwrap() else { panic!() };
        let far = p.iter().map(|v| (v[0] * v[0] + v[2] * v[2]).sqrt()).fold(0.0, f32::max);
        assert!(far >= 9000.0 && far < 9300.0);
        let Some(bevy::mesh::Indices::U32(i)) = mesh.indices() else { panic!() };
        assert_eq!(i.len() % 3, 0);
        // every triangle winds counter clockwise seen from +Y
        for t in i.chunks(3) {
            let (a, b, c) = (p[t[0] as usize], p[t[1] as usize], p[t[2] as usize]);
            let cross_y = (b[2] - a[2]) * (c[0] - a[0]) - (b[0] - a[0]) * (c[2] - a[2]);
            assert!(cross_y > 0.0, "{a:?} {b:?} {c:?}");
        }
    }

    #[test]
    fn no_foam_flag_no_foam() {
        let env = r#"<Environment><Ocean Material="x" CausticIntensity="0.5"/></Environment>"#;
        let mtl = r#"<Material MtlFlags="526336" Shader="Water" GenMask="21"><PublicParams FoamAmount="2" Tilling="3"/></Material>"#;
        let o = OceanParams::parse(env, Some(mtl));
        assert_eq!((o.foam_amount, o.tilling, o.caustic_intensity), (0.0, 3.0, 0.5));
        assert_eq!(OceanParams::parse("bad", None), OceanParams::default());
    }
}
