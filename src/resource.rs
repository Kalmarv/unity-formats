//! Streamed resources: bulk data (texture pixels, vertex data, audio) kept in a `.resS` or `.resource` file beside
//! the serialized file, named by an object's `StreamingInfo` (`offset`, `size`, `path`).
#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "reader I/O: reads streamed resource files"
)]

use std::path::{Path, PathBuf};

use crate::bundle::{Bundle, BundleNode};
use crate::value::Value;
use crate::{Error, Result, invalid};

/// Where an object's streamed data is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamingInfo {
    /// Where the data starts in the file, in bytes.
    pub offset: u64,
    /// The data's size in bytes.
    pub size: u64,
    /// The resource file as the object names it: a plain file name, or `archive:/<name>/<name>`.
    pub path: String,
}

impl StreamingInfo {
    /// From a decoded `StreamingInfo` struct; `None` when it names no data (empty path or size 0).
    pub fn of(v: &Value) -> Result<Option<StreamingInfo>> {
        let num = |k: &str| {
            v.get(k)
                .and_then(|x| match x {
                    Value::UInt(u) => Some(*u),
                    Value::Int(i) => u64::try_from(*i).ok(),
                    _ => None,
                })
                .ok_or_else(|| invalid(format!("StreamingInfo: no {k}")))
        };
        let path = v
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("StreamingInfo: no path"))?;
        let (offset, size) = (num("offset")?, num("size")?);
        if path.is_empty() || size == 0 {
            return Ok(None);
        }
        Ok(Some(StreamingInfo {
            offset,
            size,
            path: path.to_owned(),
        }))
    }

    /// The file in `dir` (the data folder) that holds the data: the path's file name (player files name it plainly or
    /// as `archive:/<cab>/<name>`).
    pub fn file_in(&self, dir: &Path) -> Result<PathBuf> {
        let name = self.path.rsplit('/').next().unwrap_or(&self.path);
        if name.is_empty() || name == "." || name == ".." || name.contains('\\') {
            return Err(invalid(format!("streamed resource path {:?}", self.path)));
        }
        Ok(dir.join(name))
    }

    /// Read the data from the file in `dir`, checking it lies inside it.
    pub fn read(&self, dir: &Path) -> Result<Vec<u8>> {
        use std::io::{Read, Seek, SeekFrom};
        let path = self.file_in(dir)?;
        let io = |source| Error::Io {
            path: path.clone(),
            source,
        };
        let mut f = std::fs::File::open(&path).map_err(io)?;
        let len = f.metadata().map_err(io)?.len();
        if self.offset.checked_add(self.size).is_none_or(|end| end > len) {
            return Err(invalid(format!(
                "{}: {}+{} past its end ({len} bytes)",
                path.display(),
                self.offset,
                self.size
            )));
        }
        f.seek(SeekFrom::Start(self.offset)).map_err(io)?;
        let size = usize::try_from(self.size).map_err(|_| {
            invalid(format!(
                "{}: {} bytes do not fit memory",
                path.display(),
                self.size
            ))
        })?;
        let mut out = vec![0; size];
        f.read_exact(&mut out).map_err(io)?;
        Ok(out)
    }
}

impl StreamingInfo {
    /// The stream node of `bundle` that the path names: the node that is no SerializedFile and whose path ends with
    /// the same last segment, compared without regard to case (the path is `archive:/<name>/<name>` for a bundled
    /// file). The data must lie inside the node; no such node, or two, is an error.
    pub fn node_in<'b>(&self, bundle: &'b Bundle) -> Result<&'b BundleNode> {
        let last = |p: &str| {
            p.replace('\\', "/")
                .rsplit('/')
                .next()
                .unwrap_or(p)
                .to_lowercase()
        };
        let name = last(&self.path);
        let mut found = bundle
            .nodes()
            .iter()
            .filter(|n| !n.is_serialized() && last(&n.path) == name);
        let (Some(node), None) = (found.next(), found.next()) else {
            return Err(invalid(format!(
                "{}: streamed resource {:?} is not one stream node of the bundle",
                bundle.path().display(),
                self.path
            )));
        };
        if self
            .offset
            .checked_add(self.size)
            .is_none_or(|end| end > node.size)
        {
            return Err(invalid(format!(
                "{}: {}: {}+{} past its end ({} bytes)",
                bundle.path().display(),
                node.path,
                self.offset,
                self.size,
                node.size
            )));
        }
        Ok(node)
    }

    /// Read the data from the node of `bundle` that [`StreamingInfo::node_in`] names.
    pub fn read_bundled(&self, bundle: &Bundle) -> Result<Vec<u8>> {
        let node = self.node_in(bundle)?;
        let len = usize::try_from(self.size)
            .map_err(|_| invalid(format!("{}: {} bytes do not fit memory", node.path, self.size)))?;
        bundle.read(node, self.offset, len)
    }
}

/// Every `StreamingInfo` struct in a decoded value (by its fields), with its path in the value.
pub fn streaming_infos(v: &Value) -> Vec<(String, &Value)> {
    fn walk<'a>(v: &'a Value, path: &str, out: &mut Vec<(String, &'a Value)>) {
        match v {
            Value::Struct(fields) => {
                if v.get("offset").is_some() && v.get("size").is_some() && v.get("path").is_some() {
                    out.push((path.to_owned(), v));
                    return;
                }
                for (k, x) in fields {
                    walk(x, &format!("{path}.{k}"), out);
                }
            }
            Value::Array(crate::value::Array::Values(items)) => {
                for (i, x) in items.iter().enumerate() {
                    walk(x, &format!("{path}[{i}]"), out);
                }
            }
            Value::Pair(p) => {
                walk(&p.0, path, out);
                walk(&p.1, path, out);
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(v, "$", &mut out);
    out
}
