//! Native helmet-driven hair/beard mesh selection from `item_assets.detail`.
//! Proven against x2game.dll AdjustHairAsset (39315820), its selection helper
//! (39315710) and body-part resolver (394E0D10). No filename-based hiding rules.
use crate::db::BodyPart;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HairVariant {
    #[default]
    Default,
    Hidden,
    Type1,
    Type2,
    Type3,
    Type4,
}

impl HairVariant {
    fn from_selector(selector: u32) -> Self {
        match selector {
            1 => Self::Hidden,
            2 => Self::Type1,
            3 => Self::Type2,
            4 => Self::Type3,
            5 => Self::Type4,
            // The native resolver falls back to the base asset for selectors 6/7.
            _ => Self::Default,
        }
    }

    fn alternate(self) -> Option<usize> {
        match self {
            Self::Type1 => Some(0),
            Self::Type2 => Some(1),
            Self::Type3 => Some(2),
            Self::Type4 => Some(3),
            _ => None,
        }
    }
}

/// Selectors from the chosen visible helmet asset, after any visual override.
/// Decode that asset's detail, rather than an item's name, armour class or slot alone.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HeadgearVariants {
    pub hair: HairVariant,
    pub beard: HairVariant,
}

impl HeadgearVariants {
    pub fn from_asset_detail(detail: u32) -> Self {
        match (detail >> 13) & 7 {
            3 | 7 => Self {
                hair: HairVariant::from_selector((detail >> 10) & 7),
                beard: HairVariant::from_selector((detail >> 7) & 7),
            },
            _ => Self::default(),
        }
    }
}

impl BodyPart {
    /// Native selected mesh ID. A missing alternate (ID zero) uses the normal mesh;
    /// explicit `Hidden` removes the attachment even when a normal mesh exists.
    pub fn variant_asset_id(&self, variant: HairVariant) -> Option<u32> {
        if variant == HairVariant::Hidden {
            return None;
        }
        Some(
            variant
                .alternate()
                .map(|i| self.asset_ids[i + 1])
                .filter(|id| *id != 0)
                .unwrap_or(self.asset_ids[0]),
        )
    }

    /// Path for the same native selection as `variant_asset_id`, retaining the original
    /// style's normal colour material. This does not alter the body-part database row.
    pub fn variant_path(&self, variant: HairVariant) -> Option<&str> {
        if variant == HairVariant::Hidden {
            return None;
        }
        Some(match variant.alternate() {
            Some(i) if self.asset_ids[i + 1] != 0 => self.hat_paths[i].as_str(),
            _ => self.path.as_str(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GameDb;

    #[test]
    fn native_selector_table_and_missing_alternates() {
        use HairVariant::*;
        // Golden results from 512 executions of the original native wrapper and the
        // original six-way resolver jump table (see hair-native/oracle.json).
        let table = [
            Default, Hidden, Type1, Type2, Type3, Type4, Default, Default,
        ];
        for kind in 0..8 {
            for hair in 0..8 {
                for beard in 0..8 {
                    let actual = HeadgearVariants::from_asset_detail(
                        (kind << 13) | (hair << 10) | (beard << 7),
                    );
                    let expected = if [3, 7].contains(&kind) {
                        HeadgearVariants {
                            hair: table[hair as usize],
                            beard: table[beard as usize],
                        }
                    } else {
                        HeadgearVariants::default()
                    };
                    assert_eq!(actual, expected);
                }
            }
        }
        let part = BodyPart {
            item_id: 0,
            slot_type: 24,
            model_id: 0,
            asset_ids: [100, 101, 102, 103, 104],
            path: "base".into(),
            hat_paths: ["1".into(), "2".into(), "3".into(), "4".into()],
            face_mask: String::new(),
            hair_base: String::new(),
        };
        let ids: Vec<_> = table.iter().map(|&v| part.variant_asset_id(v)).collect();
        assert_eq!(
            ids,
            [
                Some(100),
                None,
                Some(101),
                Some(102),
                Some(103),
                Some(104),
                Some(100),
                Some(100)
            ]
        );
        let mut missing = part;
        missing.asset_ids[1] = 0;
        missing.asset_ids[3] = 0;
        assert_eq!(missing.variant_asset_id(Type1), Some(100));
        assert_eq!(missing.variant_path(Type1), Some("base"));
        assert_eq!(missing.variant_asset_id(Type3), Some(100));
        assert_eq!(missing.variant_path(Hidden), None);
    }

    #[test]
    fn original_female_hair_has_all_four_distinct_hat_variants() {
        if !aa_cry::pak_available() {
            return;
        }
        let db = GameDb::open().unwrap();
        let helmet = db.item_visuals(22874, 11).unwrap();
        assert_eq!(helmet.len(), 1);
        assert_eq!((helmet[0].asset_id, helmet[0].detail), (10663, 28744));
        assert_eq!(db.item_meshes(22874, 11).unwrap(), [helmet[0].path.clone()]);
        assert_eq!(
            HeadgearVariants::from_asset_detail(helmet[0].detail),
            HeadgearVariants {
                hair: HairVariant::Type3,
                beard: HairVariant::Default,
            }
        );
        let part = db.body_part(24024).unwrap();
        assert_eq!(part.asset_ids, [10175, 10292, 10293, 10294, 10295]);
        assert_eq!(part.model_id, 11);
        for variant in [
            HairVariant::Default,
            HairVariant::Type1,
            HairVariant::Type2,
            HairVariant::Type3,
            HairVariant::Type4,
        ] {
            let id = part.variant_asset_id(variant).unwrap();
            let path = part.variant_path(variant).unwrap();
            assert_eq!(path, db.asset_path(id).unwrap());
            assert!(aa_pak::Pak::global().exists(path), "{path}");
        }
    }
}
