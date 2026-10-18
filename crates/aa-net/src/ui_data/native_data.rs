//! Optional native `xlcommon::Data` text codec. Opaque UI-data receipts are unchanged.
//! Comments, metadata and backslash block strings are refused atomically in this first slice.

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NativeData {
    pub children: Vec<NativeDataNode>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NativeDataNode {
    pub value: Vec<u8>,
    pub children: Vec<NativeDataNode>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativeDataError {
    Unsupported(&'static str),
    Malformed(&'static str),
    TooLarge,
}

impl NativeData {
    /// Atomic parser: a refused payload never yields a partially editable tree.
    pub fn parse(bytes: &[u8]) -> Result<Self, NativeDataError> {
        if bytes.len() > super::MAX_UI_DATA_BYTES {
            return Err(NativeDataError::TooLarge);
        }
        let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
        let mut p = Parser { bytes, pos: 0 };
        let mut lines = Vec::new();
        while p.pos < bytes.len() {
            let mut indent = 0;
            while p.peek() == Some(b' ') {
                p.pos += 1;
                indent += 1;
            }
            if p.peek() == Some(b'\t') {
                return Err(NativeDataError::Unsupported("native tab admission"));
            }
            if p.newline() {
                continue;
            }
            let nodes = p.sequence(false, 0)?;
            if !nodes.is_empty() {
                lines.push((indent, nodes));
            }
            if p.pos < bytes.len() && !p.newline() {
                return Err(NativeDataError::Malformed("expected newline"));
            }
        }
        let mut index = 0;
        let children = block(&mut lines, &mut index, 0, 0)?;
        Ok(Self { children })
    }

    /// Canonical native SaveToBuffer layout for the supported tree forms, without a NUL terminator.
    pub fn encode(&self) -> Result<Vec<u8>, NativeDataError> {
        let mut out = Vec::new();
        for node in &self.children {
            emit(node, 0, &mut out)?;
        }
        if out.len() > super::MAX_UI_DATA_BYTES {
            return Err(NativeDataError::TooLarge);
        }
        Ok(out)
    }
}

fn block(lines: &mut [(usize, Vec<NativeDataNode>)], index: &mut usize, indent: usize, depth: usize) -> Result<Vec<NativeDataNode>, NativeDataError> {
    if depth >= 127 {
        return Err(NativeDataError::Malformed("native line depth overflow"));
    }
    let mut result = Vec::new();
    while *index < lines.len() {
        let next = lines[*index].0;
        if next < indent {
            break;
        }
        if next != indent {
            return Err(NativeDataError::Unsupported("unmatched indentation level"));
        }
        let mut row = std::mem::take(&mut lines[*index].1);
        *index += 1;
        if *index < lines.len() && lines[*index].0 > indent {
            let child_indent = lines[*index].0;
            let children = block(lines, index, child_indent, depth + 1)?;
            row[0].children.extend(children);
        }
        result.extend(row);
    }
    Ok(result)
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}
impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }
    fn newline(&mut self) -> bool {
        match self.peek() {
            Some(b'\r') => {
                self.pos += 1;
                if self.peek() == Some(b'\n') {
                    self.pos += 1;
                }
                true
            }
            Some(b'\n') => {
                self.pos += 1;
                true
            }
            _ => false,
        }
    }
    fn spaces(&mut self, group: bool) -> Result<(), NativeDataError> {
        loop {
            match self.peek() {
                Some(b' ') => self.pos += 1,
                Some(b'\t') => return Err(NativeDataError::Unsupported("native tab admission")),
                Some(b'\r' | b'\n') if group => {
                    self.newline();
                }
                _ => return Ok(()),
            }
        }
    }
    fn sequence(&mut self, group: bool, depth: usize) -> Result<Vec<NativeDataNode>, NativeDataError> {
        let mut nodes = Vec::new();
        loop {
            self.spaces(group)?;
            if matches!(self.peek(), None | Some(b')' | b'\r' | b'\n')) {
                break;
            }
            if self.peek() == Some(b',') {
                return Err(NativeDataError::Malformed("missing node before comma"));
            }
            nodes.push(self.chain(group, depth)?);
            self.spaces(group)?;
            if self.peek() == Some(b',') {
                self.pos += 1;
                self.spaces(group)?;
                if matches!(self.peek(), None | Some(b')' | b'\r' | b'\n')) {
                    return Err(NativeDataError::Malformed("missing node after comma"));
                }
            } else {
                break;
            }
        }
        Ok(nodes)
    }
    fn chain(&mut self, group: bool, depth: usize) -> Result<NativeDataNode, NativeDataError> {
        if depth >= 127 {
            return Err(NativeDataError::Malformed("native tree depth overflow"));
        }
        let value = self.value()?;
        self.spaces(group)?;
        let children = match self.peek() {
            Some(b'(') => {
                self.pos += 1;
                let children = self.sequence(true, depth + 1)?;
                if self.peek() != Some(b')') {
                    return Err(NativeDataError::Malformed("unclosed group"));
                }
                self.pos += 1;
                children
            }
            None | Some(b',' | b')' | b'\r' | b'\n') => Vec::new(),
            _ => vec![self.chain(group, depth + 1)?],
        };
        Ok(NativeDataNode { value, children })
    }
    fn value(&mut self) -> Result<Vec<u8>, NativeDataError> {
        let first = self.peek().ok_or(NativeDataError::Malformed("missing value"))?;
        if first == b'#' {
            return Err(NativeDataError::Unsupported("native comments and metadata"));
        }
        if first == b'\\' {
            return Err(NativeDataError::Unsupported("native block strings"));
        }
        if matches!(first, b'(' | b')' | b',' | 0) {
            return Err(NativeDataError::Malformed("invalid value"));
        }
        let mut value = Vec::new();
        if first == b'\'' || first == b'"' {
            self.pos += 1;
            loop {
                let c = self.peek().ok_or(NativeDataError::Malformed("unclosed quote"))?;
                self.pos += 1;
                if c == first {
                    break;
                }
                if c == 0 {
                    return Err(NativeDataError::Malformed("embedded NUL"));
                }
                if c == b'\\' {
                    let next = self.peek().ok_or(NativeDataError::Malformed("unclosed escape"))?;
                    if matches!(next, b'\'' | b'"' | b'\\') {
                        self.pos += 1;
                        value.push(next);
                    } else {
                        value.push(c);
                    }
                } else if c == b'\r' {
                    if self.peek() == Some(b'\n') {
                        self.pos += 1;
                    }
                    value.push(b'\n');
                } else {
                    value.push(c);
                }
            }
        } else {
            while let Some(c) = self.peek() {
                if c <= 32 || matches!(c, b'(' | b')' | b',') {
                    break;
                }
                value.push(c);
                self.pos += 1;
            }
            if value.is_empty() {
                return Err(NativeDataError::Malformed("empty token"));
            }
        }
        Ok(value)
    }
}

fn compact(node: &NativeDataNode, depth: usize, size: &mut usize) -> bool {
    if depth >= 3 || node.value.iter().any(|b| matches!(b, b'\r' | b'\n')) {
        return false;
    }
    *size += node.value.len();
    if !node.children.is_empty() {
        *size += if node.children.len() == 1 { 1 } else { 3 };
        for (i, child) in node.children.iter().enumerate() {
            if i != 0 {
                *size += 2;
            }
            if !compact(child, depth + 1, size) {
                return false;
            }
        }
        if node.children.len() > 1 {
            *size += 2;
        }
    }
    *size < 78
}
fn value(bytes: &[u8], out: &mut Vec<u8>) -> Result<(), NativeDataError> {
    if bytes.contains(&0) {
        return Err(NativeDataError::Malformed("embedded NUL"));
    }
    if matches!(bytes.first(), Some(b'#' | b'\\')) {
        return Err(NativeDataError::Unsupported("non-roundtrippable native token"));
    }
    let quoted = bytes.is_empty() || bytes.iter().any(|b| (*b as i8) <= 32 || matches!(b, b'(' | b')' | b','));
    if !quoted && matches!(bytes.first(), Some(b'\'' | b'"')) {
        return Err(NativeDataError::Unsupported("non-roundtrippable leading quote"));
    }
    if quoted {
        out.push(b'"');
    }
    for (i, &b) in bytes.iter().enumerate() {
        if quoted && matches!(b, b'\r' | b'\n') {
            out.extend_from_slice(b"\r\n");
        } else {
            if quoted && (b == b'"' || b == b'\\' && matches!(bytes.get(i + 1), Some(b'\'' | b'"' | b'\\'))) {
                out.push(b'\\');
            }
            out.push(b);
        }
    }
    if quoted {
        out.push(b'"');
    }
    Ok(())
}
fn inline(node: &NativeDataNode, out: &mut Vec<u8>) -> Result<(), NativeDataError> {
    value(&node.value, out)?;
    if !node.children.is_empty() {
        out.extend_from_slice(if node.children.len() == 1 { b" " } else { b" ( " });
        for (i, child) in node.children.iter().enumerate() {
            if i != 0 {
                out.extend_from_slice(b", ");
            }
            inline(child, out)?;
        }
        if node.children.len() > 1 {
            out.extend_from_slice(b" )");
        }
    }
    Ok(())
}
fn emit(node: &NativeDataNode, depth: usize, out: &mut Vec<u8>) -> Result<(), NativeDataError> {
    if depth >= 127 {
        return Err(NativeDataError::Malformed("native tree depth overflow"));
    }
    out.extend(std::iter::repeat_n(b' ', depth * 4));
    if compact(node, 0, &mut 0) {
        inline(node, out)?;
        out.extend_from_slice(b"\r\n");
    } else {
        value(&node.value, out)?;
        out.extend_from_slice(b"\r\n");
        for child in &node.children {
            emit(child, depth + 1, out)?;
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn node(value: &[u8], children: Vec<NativeDataNode>) -> NativeDataNode {
        NativeDataNode {
            value: value.to_vec(),
            children,
        }
    }
    fn leaf(value: &[u8]) -> NativeDataNode {
        node(value, vec![])
    }
    #[test]
    fn native_executed_save_goldens() {
        let cases = [
            (NativeData::default(), b"".to_vec()),
            (
                NativeData {
                    children: vec![
                        node(
                            b"tracing_achievement",
                            [b"1", b"2", b"0", b"0", b"0", b"0"].into_iter().map(|v| leaf(v)).collect(),
                        ),
                        node(b"version", vec![leaf(b"1")]),
                    ],
                },
                b"tracing_achievement ( 1, 2, 0, 0, 0, 0 )\r\nversion 1\r\n".to_vec(),
            ),
            (
                NativeData {
                    children: vec![node(
                        b"outer",
                        vec![node(b"inner", vec![leaf(b"a"), leaf(b"b")]), node(b"other", vec![leaf(b"c")])],
                    )],
                },
                b"outer ( inner ( a, b ), other c )\r\n".to_vec(),
            ),
            (
                NativeData {
                    children: vec![node(
                        b"text",
                        vec![
                            leaf(b"hello world"),
                            leaf(b"quote\"x"),
                            leaf(b"back\\slash"),
                            leaf(b""),
                            leaf("\u{e9}".as_bytes()),
                        ],
                    )],
                },
                "text ( \"hello world\", quote\"x, back\\slash, \"\", \"\u{e9}\" )\r\n".as_bytes().to_vec(),
            ),
            (
                NativeData {
                    children: vec![node(b"text", vec![leaf(b"line1\nline2")])],
                },
                b"text\r\n    \"line1\r\nline2\"\r\n".to_vec(),
            ),
        ];
        for (tree, bytes) in cases {
            assert_eq!(tree.encode().unwrap(), bytes);
            assert_eq!(NativeData::parse(&bytes).unwrap(), tree);
        }
        let tree = NativeData {
            children: vec![node(&[b'a'; 80], vec![leaf(b"b")])],
        };
        let mut expected = vec![b'a'; 80];
        expected.extend_from_slice(b"\r\n    b\r\n");
        assert_eq!(tree.encode().unwrap(), expected);
        assert_eq!(NativeData::parse(&expected).unwrap(), tree);
    }
    #[test]
    fn native_load_quotes_bom_order_duplicates_and_atomic_refusal() {
        let tree = NativeData::parse(b"\xef\xbb\xbftext \"a\\qb\\\"c\"\r\nx 1\r\nx 2\r\n").unwrap();
        assert_eq!(tree.children[0].children[0].value, b"a\\qb\"c");
        assert_eq!(tree.children[1].value, tree.children[2].value);
        assert_eq!(tree.children[1].children[0].value, b"1");
        assert_eq!(tree.children[2].children[0].value, b"2");
        for input in [b"a 1\r\n# comment\r\n".as_slice(), b"#? meta\r\na 1\r\n", b"a \\block\r\n", b"a\t1\r\n"] {
            assert!(matches!(NativeData::parse(input), Err(NativeDataError::Unsupported(_))));
        }
        for input in [b"a ( 1, 2\r\n".as_slice(), b"a \"missing\r\n", b"a )\r\n", b"a\0x\r\n", b"a ( , 1 )\r\n"] {
            assert!(NativeData::parse(input).is_err());
        }
        assert_eq!(
            NativeData::parse(&vec![b'x'; super::super::MAX_UI_DATA_BYTES + 1]),
            Err(NativeDataError::TooLarge)
        );
    }
    #[test]
    fn unrelated_subtrees_survive_editing_trace_values() {
        let mut tree = NativeData::parse(b"other ( a b, c d )\r\ntracing_achievement ( 1, 0, 0, 0, 0, 0 )\r\nversion 1\r\n").unwrap();
        let before = tree.children[0].clone();
        tree.children[1].children[1].value = b"42".to_vec();
        let after = NativeData::parse(&tree.encode().unwrap()).unwrap();
        assert_eq!(after.children[0], before);
        assert_eq!(after.children[1].children[1].value, b"42");
    }
}
