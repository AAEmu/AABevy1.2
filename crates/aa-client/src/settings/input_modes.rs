//! Native393fe030 active-mode list: enable promotes, resolve walks newest first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct InputModes {
    oldest_first: Vec<u8>,
}

pub(crate) const NAMES: [&str; 5] = [
    "normal_mode",
    "mode_action_mode",
    "builder_rotate_mode",
    "screen_shot_mode",
    "quest_directing_mode",
];

impl InputModes {
    /// The original setter reads only the low enable byte. The bool result is
    /// its internal change notification, not a Lua or network receipt.
    pub(crate) fn set(&mut self, index: u32, enable: u32) -> bool {
        let Ok(index) = u8::try_from(index) else {
            return false;
        };
        if usize::from(index) >= NAMES.len() {
            return false;
        }
        let present = self.oldest_first.iter().position(|mode| *mode == index);
        if enable as u8 != 0 {
            if self.oldest_first.last() == Some(&index) {
                return false;
            }
            if let Some(position) = present {
                self.oldest_first.remove(position);
            }
            self.oldest_first.push(index);
            true
        } else if let Some(position) = present {
            self.oldest_first.remove(position);
            true
        } else {
            false
        }
    }

    pub(crate) fn priority(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.oldest_first
            .iter()
            .rev()
            .map(|index| NAMES[usize::from(*index)])
    }

    pub(crate) fn clear(&mut self) {
        self.oldest_first.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_mutation_goldens_preserve_enable_chronology_and_noops() {
        let proof: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tools/evidence/user_music_actions/input-mode-executed-cases.json"
        ))
        .unwrap();
        let mut checked = 0;
        for case in proof["cases"].as_array().unwrap() {
            if !matches!(
                case["case"].as_str(),
                Some(
                    "enable_permutation"
                        | "duplicates_and_reenable_promotes"
                        | "disable_preserves_other_modes_and_restores_previous"
                        | "range_and_low_byte_boolean"
                )
            ) {
                continue;
            }
            let Some(steps) = case["steps"].as_array() else {
                continue;
            };
            // Producer cases exercise original wrappers, while these rows carry
            // the raw original setter arguments and exact resulting list.
            let mut modes = InputModes::default();
            for step in steps {
                let Some(call) = step["call"].as_array() else {
                    continue;
                };
                let changed = modes.set(
                    call[0].as_u64().unwrap() as u32,
                    call[1].as_u64().unwrap() as u32,
                );
                let expected: Vec<_> = step["oldest_to_newest"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_u64().unwrap() as u8)
                    .collect();
                assert_eq!(modes.oldest_first, expected, "{step}");
                assert_eq!(
                    changed,
                    !step["effect_ids"].as_array().unwrap().is_empty(),
                    "{step}"
                );
                checked += 1;
            }
        }
        assert!(checked >= 35, "executed setters covered {checked}");
    }
}
