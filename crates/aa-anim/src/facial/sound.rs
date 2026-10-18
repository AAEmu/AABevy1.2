//! Sound metadata from CryAnimation.dll's sequence (315827C0) and sentence (3159DBC0)
//! XML loaders. These are references and authored data, not an audio/phoneme generator.
use super::number;
use crate::Result;

#[derive(Clone, Debug, PartialEq)]
pub struct SoundEntry {
    /// Original path, with its original case; resolve through aa-pak when playing audio.
    pub sound: String,
    /// Sequence-clock offset in seconds. Sentence time is sequence time minus this value.
    pub start: f32,
    pub sentence: Option<FacialSentence>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FacialSentence {
    pub text: String,
    pub words: Vec<SentenceWord>,
    /// Original comma-separated `start_ms:end_ms:phoneme` records, retained verbatim.
    /// Playback/blending is not implemented; the shipped FSQs have no such records.
    pub phonemes: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SentenceWord {
    pub word: String,
    pub start: f32,
    pub end: f32,
}

pub(super) fn parse(root: roxmltree::Node<'_, '_>) -> Result<Vec<SoundEntry>> {
    let mut entries = root
        .children()
        .filter(|n| n.has_tag_name("SoundEntry"))
        .map(|node| entry(node, number(node, "StartTime", 0.0)?))
        .collect::<Result<Vec<_>>>()?;
    // The native loader appends the old root-level entry after all modern entries.
    if root.has_attribute("Sound") || root.children().any(|n| n.has_tag_name("Sentence")) {
        entries.push(entry(root, 0.0)?);
    }
    Ok(entries)
}

fn entry(node: roxmltree::Node<'_, '_>, start: f32) -> Result<SoundEntry> {
    let sentence = node
        .children()
        .find(|n| n.has_tag_name("Sentence"))
        .map(|sentence| -> Result<FacialSentence> {
            let words = sentence
                .children()
                .find(|n| n.has_tag_name("Words"))
                .into_iter()
                .flat_map(|n| n.children().filter(|n| n.is_element()))
                .map(|word| {
                    Ok(SentenceWord {
                        word: word.attribute("Word").unwrap_or("").to_owned(),
                        start: number(word, "Start", 0.0)?,
                        end: number(word, "End", 0.0)?,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(FacialSentence {
                text: sentence.attribute("Text").unwrap_or("").to_owned(),
                words,
                phonemes: sentence
                    .children()
                    .find(|n| n.has_tag_name("Phonemes"))
                    .and_then(|n| n.text())
                    .unwrap_or("")
                    .to_owned(),
            })
        })
        .transpose()?;
    Ok(SoundEntry {
        sound: node.attribute("Sound").unwrap_or("").to_owned(),
        start,
        sentence,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facial::FacialSequence;

    #[test]
    fn retains_native_sound_offsets_sentence_and_legacy_order() {
        let sequence = FacialSequence::parse(
            r#"<FacialSequence Sound="Legacy.wav">
              <Sentence Text="old"/>
              <SoundEntry Sound="Game/Voice/Line.wav" StartTime="-0.25">
                <Sentence Text="Hello &amp; goodbye">
                  <Words><Word Word="Hello" Start="0.1" End="0.4"/></Words>
                  <Phonemes>100:200:AA,200:400:M</Phonemes>
                </Sentence>
              </SoundEntry><SoundEntry/>
            </FacialSequence>"#,
        )
        .unwrap();
        assert_eq!(sequence.sound_entries, 3);
        assert_eq!(sequence.sounds[0].sound, "Game/Voice/Line.wav");
        assert_eq!(sequence.sounds[0].start, -0.25);
        assert_eq!(
            sequence.sounds[0].sentence,
            Some(FacialSentence {
                text: "Hello & goodbye".into(),
                words: vec![SentenceWord {
                    word: "Hello".into(),
                    start: 0.1,
                    end: 0.4
                }],
                phonemes: "100:200:AA,200:400:M".into(),
            })
        );
        assert_eq!(sequence.sounds[1].start, 0.0);
        assert_eq!(sequence.sounds[1].sentence, None);
        assert_eq!(sequence.sounds[2].sound, "Legacy.wav");
        assert_eq!(sequence.sounds[2].start, 0.0);
        assert!(
            FacialSequence::parse(
                r#"<FacialSequence><SoundEntry StartTime="NaN"/></FacialSequence>"#
            )
            .is_err()
        );
    }

    #[test]
    fn root_sentence_without_sound_still_creates_a_legacy_entry() {
        let sequence =
            FacialSequence::parse(r#"<FacialSequence><Sentence Text="silent"/></FacialSequence>"#)
                .unwrap();
        assert_eq!(sequence.sound_entries, 1);
        assert_eq!(sequence.sounds[0].sound, "");
        assert_eq!(sequence.sounds[0].sentence.as_ref().unwrap().text, "silent");
    }
}
