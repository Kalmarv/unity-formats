//! Type trees: a class's serialized layout, and decoding an object's bytes by it.
//!
//! The rules: primitives by their type name; `string` is a 32-bit length, the UTF-8 bytes, then 4-byte alignment;
//! `TypelessData` a 32-bit length and the bytes; `pair` its two children; a node whose first child is `Array` is a
//! vector (a 32-bit count, then the elements, each by the array's second child); anything else is a struct of its
//! children in order. After any node whose meta flag has `ALIGN`, the position is aligned to 4 bytes (from the
//! object's start); an `Array` child's flag aligns its vector.

use std::sync::Arc;

use crate::reader::Reader;
use crate::value::{Array, Value};
use crate::{Error, Result, invalid};

/// The meta flag that aligns the position to 4 bytes after the node.
pub const ALIGN: u32 = 0x4000;

/// One node: its type and field name, its byte size (-1: variable), version, meta flags and children (indices into
/// the tree's node list).
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    /// The node's type name (`int`, `string`, `vector`, a class name, ...).
    pub ty: Arc<str>,
    /// The field name.
    pub name: Arc<str>,
    /// The size in bytes the tree states for a value of this node, or -1 where it varies.
    pub byte_size: i32,
    /// The node's version, as stored.
    pub version: i32,
    /// The meta flags: [`ALIGN`] and others the decoder does not use.
    pub meta_flag: u32,
    /// Indices into [`TypeTree::nodes`], in field order; every child's index is greater than its parent's.
    pub children: Vec<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Prim {
    Bool,
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    F32,
    F64,
}

impl Prim {
    fn of(ty: &str) -> Option<Prim> {
        Some(match ty {
            "bool" => Prim::Bool,
            "SInt8" => Prim::I8,
            "UInt8" | "char" => Prim::U8,
            "short" | "SInt16" => Prim::I16,
            "unsigned short" | "UInt16" => Prim::U16,
            "int" | "SInt32" => Prim::I32,
            "unsigned int" | "UInt32" | "Type*" => Prim::U32,
            "long long" | "SInt64" => Prim::I64,
            "unsigned long long" | "UInt64" | "FileSize" => Prim::U64,
            "float" => Prim::F32,
            "double" => Prim::F64,
            _ => return None,
        })
    }

    fn size(self) -> usize {
        match self {
            Prim::Bool | Prim::I8 | Prim::U8 => 1,
            Prim::I16 | Prim::U16 => 2,
            Prim::I32 | Prim::U32 | Prim::F32 => 4,
            Prim::I64 | Prim::U64 | Prim::F64 => 8,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Kind {
    Prim(Prim),
    String,
    Typeless,
    Pair,
    /// The element node; whether the `Array` child aligns.
    Vector {
        element: usize,
        aligned: bool,
    },
    /// A managed reference (`[SerializeReference]`, a `ReferencedObject`): its id and class, then its data, laid out by
    /// the type tree the file stores for that class ([`ReferenceTrees`]).
    Referenced,
    /// The managed-reference registry (`ManagedReferencesRegistry`): stored once, by the object; one inside a
    /// referenced object's data is not stored.
    Registry,
    Struct,
}

/// Where a managed reference's data is laid out: the type tree a file stores for the reference type of a class (its name,
/// namespace and assembly). [`crate::SerializedFile`] gives its reference types'.
pub trait ReferenceTrees {
    /// The tree of the class, `None` when the file stores none for it.
    fn reference_tree(&self, class: &str, namespace: &str, assembly: &str) -> Option<&TypeTree>;
}

/// No reference trees: a managed reference with a class is refused.
struct NoReferences;

impl ReferenceTrees for NoReferences {
    fn reference_tree(&self, _: &str, _: &str, _: &str) -> Option<&TypeTree> {
        None
    }
}

/// What a read needs besides the bytes: the reference trees, and whether it is inside the registry (where a nested one
/// is not stored).
#[derive(Clone, Copy)]
struct Context<'a> {
    references: &'a dyn ReferenceTrees,
    in_registry: bool,
}

/// A class's type tree; node 0 is the root.
#[derive(Clone, Debug, PartialEq)]
pub struct TypeTree {
    /// The nodes, the root first.
    pub nodes: Vec<Node>,
    kinds: Vec<Kind>,
    /// Per node, the fewest bytes a value of it takes (alignment left out): what bounds a vector's count by the bytes
    /// left.
    min_sizes: Vec<usize>,
}

/// The most elements a vector of a type that takes no bytes (a struct without fields) may state: its elements cost
/// nothing to the bytes, so the bytes cannot bound its count.
const MAX_EMPTY_ELEMENTS: usize = 1 << 16;

impl TypeTree {
    /// Build a tree from its nodes (the root first). Refused: no nodes, a child index that is not after its parent's
    /// or not in the list, a `pair` without two children, or a vector whose `Array` child does not have two children.
    pub fn new(nodes: Vec<Node>) -> Result<Self> {
        if nodes.is_empty() {
            return Err(invalid("empty type tree"));
        }
        let mut kinds = Vec::with_capacity(nodes.len());
        for (i, n) in nodes.iter().enumerate() {
            if n.children.iter().any(|&c| c <= i || c >= nodes.len()) {
                return Err(invalid(format!("type tree node {i} has a bad child index")));
            }
            let first = n.children.first().and_then(|&c| nodes.get(c));
            let kind = if let Some(p) = Prim::of(&n.ty) {
                Kind::Prim(p)
            } else if &*n.ty == "string" {
                Kind::String
            } else if &*n.ty == "TypelessData" {
                Kind::Typeless
            } else if &*n.ty == "pair" {
                if n.children.len() != 2 {
                    return Err(invalid(format!(
                        "pair node {i} has {} children",
                        n.children.len()
                    )));
                }
                Kind::Pair
            } else if &*n.ty == "ReferencedObject" {
                Kind::Referenced
            } else if &*n.ty == "ManagedReferencesRegistry" {
                Kind::Registry
            } else if let Some(a) = first.filter(|f| &*f.ty == "Array") {
                let &[_, element] = a.children.as_slice() else {
                    return Err(invalid(format!(
                        "Array node {} has {} children",
                        n.children.first().copied().unwrap_or_default(),
                        a.children.len()
                    )));
                };
                Kind::Vector {
                    element,
                    aligned: a.meta_flag & ALIGN != 0,
                }
            } else {
                Kind::Struct
            };
            kinds.push(kind);
        }
        // children come after their parent, so the sizes are summed from the last node up
        let mut min_sizes = vec![0usize; nodes.len()];
        for (i, (node, kind)) in nodes.iter().zip(&kinds).enumerate().rev() {
            let size = match kind {
                Kind::Prim(p) => p.size(),
                Kind::String | Kind::Typeless | Kind::Vector { .. } => 4,
                Kind::Pair | Kind::Struct | Kind::Registry => node
                    .children
                    .iter()
                    .map(|&c| min_sizes.get(c).copied().unwrap_or(0))
                    .fold(0, usize::saturating_add),
                Kind::Referenced => 0,
            };
            if let Some(slot) = min_sizes.get_mut(i) {
                *slot = size;
            }
        }
        Ok(TypeTree {
            nodes,
            kinds,
            min_sizes,
        })
    }

    /// The root node; `None` only when [`TypeTree::nodes`] was emptied after [`TypeTree::new`] refused an empty tree.
    pub fn root(&self) -> Option<&Node> {
        self.nodes.first()
    }

    /// Node `i` and its kind: an error when the tree no longer holds the node (`nodes` is public, `new` checked it).
    fn node(&self, i: usize) -> Result<(&Node, &Kind)> {
        self.nodes
            .get(i)
            .zip(self.kinds.get(i))
            .ok_or_else(|| invalid(format!("type tree node {i} of {}", self.nodes.len())))
    }

    /// Decode `data` (an object's bytes) from its start; returns the value and the bytes consumed. A managed reference
    /// with a class is refused: [`TypeTree::decode_with_references`] reads it.
    pub fn decode(&self, data: &[u8], big_endian: bool) -> Result<(Value, usize)> {
        self.decode_with_references(data, big_endian, &NoReferences)
    }

    /// As [`TypeTree::decode`], reading each managed reference's data by the tree `references` gives its class (a null
    /// reference, of no class, has none). A class it has no tree for is refused.
    pub fn decode_with_references(
        &self,
        data: &[u8],
        big_endian: bool,
        references: &dyn ReferenceTrees,
    ) -> Result<(Value, usize)> {
        let mut r = Reader::new(data, big_endian);
        let context = Context {
            references,
            in_registry: false,
        };
        let v = self.read(0, &mut r, 0, context)?;
        Ok((v, r.pos))
    }

    /// The root struct's field `name` alone: the fields before it are walked (and so must decode), the ones after it
    /// are not read, so a name stored first costs a few bytes whatever the object holds. `None` when the root has no
    /// such field; the value is what [`TypeTree::decode`] gives it. A root that is not a struct has no fields.
    pub fn decode_field(&self, data: &[u8], big_endian: bool, name: &str) -> Result<Option<Value>> {
        self.decode_field_with_references(data, big_endian, name, &NoReferences)
    }

    /// As [`TypeTree::decode_field`], reading managed references as [`TypeTree::decode_with_references`] does.
    pub fn decode_field_with_references(
        &self,
        data: &[u8],
        big_endian: bool,
        name: &str,
        references: &dyn ReferenceTrees,
    ) -> Result<Option<Value>> {
        let (root, kind) = self.node(0)?;
        if !matches!(kind, Kind::Struct) {
            return Ok(None);
        }
        let mut r = Reader::new(data, big_endian);
        let context = Context {
            references,
            in_registry: false,
        };
        for &c in &root.children {
            let v = self.read(c, &mut r, 1, context)?;
            if &*self.node(c)?.0.name == name {
                return Ok(Some(v));
            }
        }
        Ok(None)
    }

    fn read(&self, i: usize, r: &mut Reader, depth: u32, context: Context) -> Result<Value> {
        if depth > 128 {
            return Err(invalid("type tree recursion deeper than 128"));
        }
        let (n, kind) = self.node(i)?;
        let v = match kind {
            Kind::Prim(p) => prim(*p, r)?,
            Kind::String => {
                let len = length(r, 1)?;
                let b = r.take(len)?.to_vec();
                r.align(4)?;
                String::from_utf8(b).map_or_else(|e| Value::Bytes(e.into_bytes()), Value::String)
            }
            Kind::Typeless => {
                let len = length(r, 1)?;
                Value::Array(Array::U8(r.take(len)?.to_vec()))
            }
            Kind::Pair => {
                let &[first, second] = n.children.as_slice() else {
                    return Err(invalid(format!(
                        "pair node {i} has {} children",
                        n.children.len()
                    )));
                };
                let a = self.read(first, r, depth.saturating_add(1), context)?;
                let b = self.read(second, r, depth.saturating_add(1), context)?;
                Value::Pair(Box::new((a, b)))
            }
            Kind::Vector { element, aligned } => {
                let v = self.vector(*element, r, depth, context)?;
                if *aligned {
                    r.align(4)?;
                }
                v
            }
            Kind::Referenced => self.referenced(n, r, depth, context)?,
            Kind::Registry if context.in_registry => return Ok(Value::Struct(Vec::new())),
            Kind::Struct | Kind::Registry => {
                let context = Context {
                    in_registry: context.in_registry || matches!(kind, Kind::Registry),
                    ..context
                };
                let mut fields = Vec::with_capacity(n.children.len());
                for &c in &n.children {
                    fields.push((
                        self.node(c)?.0.name.clone(),
                        self.read(c, r, depth.saturating_add(1), context)?,
                    ));
                }
                Value::Struct(fields)
            }
        };
        if n.meta_flag & ALIGN != 0 {
            r.align(4)?;
        }
        Ok(v)
    }

    /// A `ReferencedObject`: its fields in order, its `ReferencedObjectData` read by the tree of the class its `type`
    /// names (left out for a null reference, whose class is empty), that tree's own registry not stored.
    fn referenced(&self, n: &Node, r: &mut Reader, depth: u32, context: Context) -> Result<Value> {
        let mut fields: Vec<(Arc<str>, Value)> = Vec::with_capacity(n.children.len());
        for &c in &n.children {
            let child = self.node(c)?.0;
            if &*child.ty != "ReferencedObjectData" {
                fields.push((
                    child.name.clone(),
                    self.read(c, r, depth.saturating_add(1), context)?,
                ));
                continue;
            }
            let ty = fields
                .iter()
                .find(|(k, _)| &**k == "type")
                .map(|(_, v)| v)
                .ok_or_else(|| invalid(format!("{}: a managed reference's data before its type", n.name)))?;
            let part = |k: &str| {
                ty.get(k)
                    .and_then(Value::as_str)
                    .ok_or_else(|| invalid(format!("{}: a managed reference's type without its {k}", n.name)))
            };
            let (class, namespace, assembly) = (part("class")?, part("ns")?, part("asm")?);
            if class.is_empty() {
                continue;
            }
            let tree = context.references.reference_tree(class, namespace, assembly).ok_or_else(|| {
                Error::Unsupported(format!(
                    "{}: a managed reference of class {namespace}.{class} ({assembly}), whose type tree the file does \
                     not store",
                    n.name
                ))
            })?;
            let data = tree.read(
                0,
                r,
                depth.saturating_add(1),
                Context {
                    in_registry: true,
                    ..context
                },
            )?;
            fields.push((child.name.clone(), data));
        }
        Ok(Value::Struct(fields))
    }

    fn vector(&self, e: usize, r: &mut Reader, depth: u32, context: Context) -> Result<Value> {
        let (en, kind) = self.node(e)?;
        // a primitive element without its own alignment: the elements packed
        if let Kind::Prim(p) = *kind
            && en.meta_flag & ALIGN == 0
        {
            let n = length(r, p.size())?;
            return Ok(Value::Array(prim_array(p, n, r)?));
        }
        // the shortest an element can be, to bound the count by the bytes left
        let min = self.min_sizes.get(e).copied().unwrap_or(0);
        let n = length(r, min)?;
        if min == 0 && n > MAX_EMPTY_ELEMENTS {
            return Err(invalid(format!(
                "a vector of {n} elements that take no bytes (at most {MAX_EMPTY_ELEMENTS})"
            )));
        }
        let mut out = Vec::with_capacity(n.min(1 << 16));
        for _ in 0..n {
            out.push(self.read(e, r, depth.saturating_add(1), context)?);
        }
        Ok(Value::Array(Array::Values(out)))
    }
}

/// A 32-bit length or count, checked against the bytes left at `unit` bytes per element.
fn length(r: &mut Reader, unit: usize) -> Result<usize> {
    let at = r.pos;
    let n = r.i32()?;
    let n = usize::try_from(n).map_err(|_| invalid(format!("negative length {n} at offset {at}")))?;
    if n.saturating_mul(unit) > r.remaining() {
        return Err(invalid(format!(
            "length {n} at offset {at} runs past the object ({} bytes left)",
            r.remaining()
        )));
    }
    Ok(n)
}

fn prim(p: Prim, r: &mut Reader) -> Result<Value> {
    Ok(match p {
        Prim::Bool => Value::Bool(r.bool()?),
        Prim::I8 => Value::Int(i64::from(r.i8()?)),
        Prim::U8 => Value::UInt(u64::from(r.u8()?)),
        Prim::I16 => Value::Int(i64::from(r.i16()?)),
        Prim::U16 => Value::UInt(u64::from(r.u16()?)),
        Prim::I32 => Value::Int(i64::from(r.i32()?)),
        Prim::U32 => Value::UInt(u64::from(r.u32()?)),
        Prim::I64 => Value::Int(r.i64()?),
        Prim::U64 => Value::UInt(r.u64()?),
        Prim::F32 => Value::Float(r.f32()?),
        Prim::F64 => Value::Double(r.f64()?),
    })
}

fn prim_array(p: Prim, n: usize, r: &mut Reader) -> Result<Array> {
    macro_rules! many {
        ($f:ident, $v:ident) => {
            Array::$v((0..n).map(|_| r.$f()).collect::<Result<Vec<_>>>()?)
        };
    }
    Ok(match p {
        Prim::U8 => Array::U8(r.take(n)?.to_vec()),
        Prim::Bool => many!(bool, Bool),
        Prim::I8 => many!(i8, I8),
        Prim::I16 => many!(i16, I16),
        Prim::U16 => many!(u16, U16),
        Prim::I32 => many!(i32, I32),
        Prim::U32 => many!(u32, U32),
        Prim::I64 => many!(i64, I64),
        Prim::U64 => many!(u64, U64),
        Prim::F32 => many!(f32, F32),
        Prim::F64 => many!(f64, F64),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(ty: &str, name: &str, meta: u32, children: Vec<usize>) -> Node {
        Node {
            ty: ty.into(),
            name: name.into(),
            byte_size: -1,
            version: 1,
            meta_flag: meta,
            children,
        }
    }

    /// A vector's count is bounded by the bytes left at the fewest bytes an element takes: a struct of fields takes the
    /// sum of theirs, and elements that take none (a struct without fields) are capped by a constant.
    #[test]
    fn a_vectors_count_is_bounded_by_what_its_elements_take() {
        let vector_of = |element: Vec<Node>| {
            let mut nodes = vec![
                node("Thing", "Base", 0, vec![1]),
                node("vector", "m_V", 0, vec![2]),
                node("Array", "Array", 0, vec![3, 4]),
                node("int", "size", 0, vec![]),
            ];
            let first = nodes.len();
            for (k, mut n) in element.into_iter().enumerate() {
                n.children = n.children.iter().map(|c| c + first).collect();
                if k == 0 {
                    n.name = "data".into();
                }
                nodes.push(n);
            }
            TypeTree::new(nodes).unwrap()
        };
        let count = |n: i32| n.to_le_bytes();
        // elements of a struct of two ints take 8 bytes: a count the bytes cannot hold is refused before anything is
        // made of it
        let pairs = vector_of(vec![
            node("Two", "data", 0, vec![1, 2]),
            node("int", "a", 0, vec![]),
            node("int", "b", 0, vec![]),
        ]);
        let e = pairs
            .decode(&count(1_000_000_000), false)
            .unwrap_err()
            .to_string();
        assert!(e.contains("runs past the object"), "{e}");
        let mut two = count(2).to_vec();
        two.extend([0u8; 16]);
        assert!(pairs.decode(&two, false).is_ok());
        // elements of no bytes: some are fine, billions are not
        let empty = vector_of(vec![node("Empty", "data", 0, vec![])]);
        assert!(empty.decode(&count(3), false).is_ok());
        let e = empty.decode(&count(i32::MAX), false).unwrap_err().to_string();
        assert!(e.contains("take no bytes"), "{e}");
    }

    /// A struct of a bool, an aligned string, a vector of floats and a pair, decoded and aligned as the rules say.
    #[test]
    fn decodes_by_the_rules() {
        let t = TypeTree::new(vec![
            node("Thing", "Base", 0, vec![1, 2, 6, 10]),
            node("bool", "m_On", ALIGN, vec![]),
            node("string", "m_Name", 0, vec![3]),
            node("Array", "Array", ALIGN, vec![4, 5]),
            node("int", "size", 0, vec![]),
            node("char", "data", 0, vec![]),
            node("vector", "m_V", 0, vec![7]),
            node("Array", "Array", 0, vec![8, 9]),
            node("int", "size", 0, vec![]),
            node("float", "data", 0, vec![]),
            node("pair", "m_P", 0, vec![11, 12]),
            node("int", "first", 0, vec![]),
            node("UInt8", "second", ALIGN, vec![]),
        ])
        .unwrap();
        let mut d = vec![1, 0, 0, 0];
        d.extend_from_slice(&3i32.to_le_bytes());
        d.extend_from_slice(b"abc\0");
        d.extend_from_slice(&2i32.to_le_bytes());
        d.extend_from_slice(&1.5f32.to_le_bytes());
        d.extend_from_slice(&(-2.0f32).to_le_bytes());
        d.extend_from_slice(&7i32.to_le_bytes());
        d.extend_from_slice(&[9, 0, 0, 0]);
        let (v, used) = t.decode(&d, false).unwrap();
        assert_eq!(used, d.len());
        assert_eq!(v.get("m_On"), Some(&Value::Bool(true)));
        assert_eq!(v.get("m_Name").and_then(Value::as_str), Some("abc"));
        assert_eq!(v.get("m_V"), Some(&Value::Array(Array::F32(vec![1.5, -2.0]))));
        assert_eq!(
            v.get("m_P"),
            Some(&Value::Pair(Box::new((Value::Int(7), Value::UInt(9)))))
        );
        // one field alone: what decode gives it, the fields after it unread (a broken tail does not matter), none for a
        // name the root lacks
        for name in ["m_On", "m_Name", "m_V", "m_P"] {
            assert_eq!(
                t.decode_field(&d, false, name).unwrap().as_ref(),
                v.get(name),
                "{name}"
            );
        }
        assert_eq!(t.decode_field(&d, false, "m_Missing").unwrap(), None);
        assert_eq!(
            t.decode_field(&d[..12], false, "m_Name")
                .unwrap()
                .as_ref()
                .and_then(Value::as_str),
            Some("abc"),
            "the name decodes from the bytes up to it"
        );
        assert!(t.decode_field(&d[..10], false, "m_Name").is_err());
        // a count past the data and a bad bool are errors
        let mut short = d.clone();
        short[12..16].copy_from_slice(&1000i32.to_le_bytes());
        assert!(t.decode(&short, false).is_err());
        let mut b = d.clone();
        b[0] = 2;
        assert!(t.decode(&b, false).is_err());
    }

    /// A string is Unity's byte string: one whose bytes are not UTF-8 (a TextAsset's binary `m_Script`) decodes to
    /// its bytes, aligned as a string is, and the fields after it read.
    #[test]
    fn a_string_that_is_not_utf8_is_its_bytes() {
        let t = TypeTree::new(vec![
            node("TextAsset", "Base", 0, vec![1, 5]),
            node("string", "m_Script", 0, vec![2]),
            node("Array", "Array", ALIGN, vec![3, 4]),
            node("int", "size", 0, vec![]),
            node("char", "data", 0, vec![]),
            node("int", "m_After", 0, vec![]),
        ])
        .unwrap();
        let mut d = 5i32.to_le_bytes().to_vec();
        d.extend_from_slice(&[0x89, b'P', 0xff, 0x00, 0xc3, 0, 0, 0]);
        d.extend_from_slice(&42i32.to_le_bytes());
        let (v, used) = t.decode(&d, false).unwrap();
        assert_eq!(used, d.len());
        let script = v.get("m_Script").unwrap();
        assert_eq!(script, &Value::Bytes(vec![0x89, b'P', 0xff, 0x00, 0xc3]));
        assert_eq!(script.as_str(), None);
        assert_eq!(script.as_bytes(), Some(&[0x89, b'P', 0xff, 0x00, 0xc3][..]));
        assert_eq!(v.get("m_After"), Some(&Value::Int(42)));
    }

    /// Nodes of a `string` named `name` at index `at`: the string, its `Array` (aligned) and the size and char nodes.
    fn string_nodes(name: &str, at: usize) -> Vec<Node> {
        vec![
            node("string", name, 0, vec![at + 1]),
            node("Array", "Array", ALIGN, vec![at + 2, at + 3]),
            node("int", "size", 0, vec![]),
            node("char", "data", 0, vec![]),
        ]
    }

    fn string_bytes(d: &mut Vec<u8>, s: &str) {
        d.extend_from_slice(&(s.len() as i32).to_le_bytes());
        d.extend_from_slice(s.as_bytes());
        while !d.len().is_multiple_of(4) {
            d.push(0);
        }
    }

    /// The trees of one class, `Thing`: an int and a registry of its own (which a referenced object does not store).
    struct Things(TypeTree);

    impl ReferenceTrees for Things {
        fn reference_tree(&self, class: &str, namespace: &str, assembly: &str) -> Option<&TypeTree> {
            ((class, namespace, assembly) == ("Thing", "Game", "Assembly-CSharp")).then_some(&self.0)
        }
    }

    /// A registry of version 2 with two references: one of class `Thing`, its data read by the class's tree (that
    /// tree's own registry not stored), and a null one (no class), which has no data; then the field after the
    /// registry. Without the trees, or with a class they do not have, the reference is refused.
    #[test]
    fn managed_references_read_by_their_class_trees() {
        // MonoBehaviour { ManagedReferencesRegistry references { int version; vector RefIds { Array { int size;
        // ReferencedObject data { SInt64 rid; ReferencedManagedType type { class, ns, asm }; ReferencedObjectData data
        // } } } }; int m_After }
        let mut nodes = vec![
            node("MonoBehaviour", "Base", 0, vec![1, 22]),
            node("ManagedReferencesRegistry", "references", 0, vec![2, 3]),
            node("int", "version", 0, vec![]),
            node("vector", "RefIds", 0, vec![4]),
            node("Array", "Array", 0, vec![5, 6]),
            node("int", "size", 0, vec![]),
            node("ReferencedObject", "data", 0, vec![7, 8, 21]),
            node("SInt64", "rid", 0, vec![]),
            node("ReferencedManagedType", "type", 0, vec![9, 13, 17]),
        ];
        nodes.extend(string_nodes("class", 9));
        nodes.extend(string_nodes("ns", 13));
        nodes.extend(string_nodes("asm", 17));
        nodes.push(node("ReferencedObjectData", "data", 0, vec![]));
        nodes.push(node("int", "m_After", 0, vec![]));
        let mono = TypeTree::new(nodes).unwrap();
        let thing = TypeTree::new(vec![
            node("Thing", "Base", 0, vec![1, 2]),
            node("int", "m_X", 0, vec![]),
            node("ManagedReferencesRegistry", "references", 0, vec![3]),
            node("int", "version", 0, vec![]),
        ])
        .unwrap();
        let mut d = 2i32.to_le_bytes().to_vec();
        d.extend_from_slice(&2i32.to_le_bytes());
        d.extend_from_slice(&7i64.to_le_bytes());
        for s in ["Thing", "Game", "Assembly-CSharp"] {
            string_bytes(&mut d, s);
        }
        d.extend_from_slice(&42i32.to_le_bytes());
        d.extend_from_slice(&(-2i64).to_le_bytes());
        for s in ["", "", ""] {
            string_bytes(&mut d, s);
        }
        d.extend_from_slice(&9i32.to_le_bytes());
        let things = Things(thing);
        let (v, used) = mono.decode_with_references(&d, false, &things).unwrap();
        assert_eq!(used, d.len());
        assert_eq!(v.get("m_After"), Some(&Value::Int(9)));
        let Some(Value::Array(Array::Values(ids))) = v.at(&["references", "RefIds"]) else {
            panic!("no RefIds: {v:?}");
        };
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0].get("rid"), Some(&Value::Int(7)));
        assert_eq!(ids[0].at(&["data", "m_X"]), Some(&Value::Int(42)));
        assert_eq!(
            ids[0].at(&["data", "references"]),
            Some(&Value::Struct(Vec::new()))
        );
        assert_eq!(ids[1].get("rid"), Some(&Value::Int(-2)));
        assert_eq!(ids[1].get("data"), None);
        // without the trees, or with another class's, the reference is refused by its class
        for refusal in [mono.decode(&d, false).unwrap_err(), {
            let mut other = d.clone();
            other[20] = b'W';
            mono.decode_with_references(&other, false, &things).unwrap_err()
        }] {
            assert!(matches!(refusal, Error::Unsupported(_)), "{refusal}");
            assert!(
                refusal
                    .to_string()
                    .contains("whose type tree the file does not store"),
                "{refusal}"
            );
        }
    }
}
