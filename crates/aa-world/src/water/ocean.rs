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
