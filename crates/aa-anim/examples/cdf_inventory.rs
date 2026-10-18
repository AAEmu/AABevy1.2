//! Read-only original CDF attachment inventory; no geometry extraction or asset writes.
use std::collections::BTreeMap;

fn main() {
    let pak = aa_pak::Pak::global();
    let mut counts = BTreeMap::<(String, String), (usize, usize, usize)>::new();
    let mut files = 0;
    let mut failures = Vec::new();
    let mut cases = Vec::new();
    let mut bindings = BTreeMap::<(String, String), (usize, usize)>::new();
    let mut payloads = BTreeMap::<String, (usize, usize)>::new();
    for path in pak
        .names()
        .filter(|path| path.to_ascii_lowercase().ends_with(".cdf"))
    {
        files += 1;
        let bytes = pak.read(path).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        let Ok(doc) = roxmltree::Document::parse(text.trim_start_matches('\u{feff}')) else {
            assert!(
                !text.contains("<Attachment"),
                "unexamined attachment XML: {path}"
            );
            failures.push(path.to_string());
            continue;
        };
        for attachment in doc
            .descendants()
            .filter(|node| node.has_tag_name("Attachment"))
        {
            let kind = attachment.attribute("Type").unwrap_or("");
            let alignment = attachment.attribute("AlignBoneAttachment").unwrap_or("");
            let binding = attachment.attribute("Binding").unwrap_or("");
            let flags = attachment
                .attribute("Flags")
                .unwrap_or("0")
                .parse::<u32>()
                .unwrap_or(0);
            let payload = payloads
                .entry(attachment.attribute("AttachmentType").unwrap_or("").into())
                .or_default();
            payload.0 += 1;
            payload.1 += usize::from(!binding.is_empty() && flags & 1 == 0);
            let count = counts.entry((kind.into(), alignment.into())).or_default();
            count.0 += 1;
            count.1 += usize::from(!binding.is_empty());
            count.2 += usize::from(!binding.is_empty() && flags & 1 == 0);
            if !binding.is_empty() && flags & 1 == 0 {
                let ext = binding
                    .rsplit('.')
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase();
                let count = bindings.entry((kind.into(), ext.clone())).or_default();
                count.0 += 1;
                count.1 += usize::from(pak.exists(binding));
                if kind == "CA_BONE" && ext != "cgf" {
                    cases.push(format!(
                        "BONE_ASSET\t{path}\t{}\t{alignment}\t{binding}\texists={}",
                        attachment.attribute("AName").unwrap_or(""),
                        pak.exists(binding)
                    ));
                }
            }
            if (kind != "CA_SKIN" && kind != "CA_BONE")
                || alignment.eq_ignore_ascii_case("Skin")
                || attachment
                    .attribute("AttachmentType")
                    .is_some_and(|kind| kind.eq_ignore_ascii_case("entity"))
            {
                cases.push(format!(
                    "{path}\t{}\t{kind}\t{alignment}\t{flags}\t{binding}\t{}",
                    attachment.attribute("AName").unwrap_or(""),
                    &text[attachment.range()]
                ));
            }
        }
    }
    assert_eq!((files, failures.len()), (13701, 16));
    assert_eq!(
        counts,
        BTreeMap::from([
            (("CA_BONE".into(), "".into()), (348, 203, 203)),
            (("CA_BONE".into(), "Bone".into()), (1118, 119, 117)),
            (("CA_SKIN".into(), "".into()), (939, 153, 148)),
        ])
    );
    assert_eq!(
        payloads,
        BTreeMap::from([
            ("".into(), (2191, 254)),
            ("effect".into(), (211, 211)),
            ("entity".into(), (3, 3)),
        ])
    );
    println!("CDF files={files} XML failures={failures:?}");
    for ((kind, alignment), (total, bound, visible)) in counts {
        println!("COUNT\t{kind}\t{alignment}\ttotal={total}\tbound={bound}\tvisible={visible}");
    }
    for ((kind, ext), (bound, exists)) in bindings {
        println!("BIND\t{kind}\t{ext}\tvisible={bound}\texists={exists}");
    }
    for (kind, (total, visible)) in payloads {
        println!("PAYLOAD\t{kind}\ttotal={total}\tvisible={visible}");
    }
    cases.sort();
    for case in cases {
        println!("CASE\t{case}");
    }
}
