use alloc::{
    string::{String, ToString},
    sync::Arc,
    vec::Vec,
};
use core::fmt::Error;

use systype::{SysError, SysResult, SyscallResult};

use crate::{dentry, Dentry, InodeMode, OpenFlags};

#[derive(Clone)]
pub struct Path {
    /// The root of the file system
    root: Arc<dyn Dentry>,
    /// The directory to start searching from
    start: Arc<dyn Dentry>,
    /// The path to search for
    path: String,
}

impl Eq for Path {}

impl PartialEq for Path {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path && Arc::ptr_eq(&self.start, &other.start)
    }
}

impl Path {
    pub fn new(root: Arc<dyn Dentry>, start: Arc<dyn Dentry>, path: &str) -> Self {
        Self {
            root,
            start,
            path: path.to_string(),
        }
    }

    /// Walk until path has been resolved.
    pub fn walk(&self, mode: InodeMode) -> SysResult<Arc<dyn Dentry>> {
        let path = self.path.as_str();
        let mut dentry = if is_absolute_path(path) {
            self.root.clone()
        } else {
            self.start.clone()
        };
        log::debug!("[Path::walk] {:?}", split_path(path));
        for p in split_path(path) {
            match p {
                ".." => {
                    dentry = dentry.parent().ok_or(SysError::ENOENT)?;
                }
                // NOTE: lookup will only create negative dentry in non-negetive dir dentry
                name => match dentry.lookup(name) {
                    Ok(sub_dentry) => {
                        log::debug!("[Path::walk] sub dentry {}", sub_dentry.name());
                        dentry = sub_dentry
                    }
                    Err(e) => {
                        log::error!("[Path::walk] {e:?} when walking in path {path}");
                        return Err(e);
                    }
                },
            }
        }
        Ok(dentry)
    }
}

pub fn is_absolute_path(path: &str) -> bool {
    path.starts_with('/')
}

pub fn is_relative_path(path: &str) -> bool {
    !path.starts_with('/')
}

pub fn split_path(path: &str) -> Vec<&str> {
    path.split('/')
        .filter(|name| !name.is_empty() && *name != ".")
        .collect()
}

/// # Example
///
/// "/" -> "/"
/// "/dir/" -> "dir"
/// "/dir/file" -> "file"
pub fn get_name(path: &str) -> &str {
    path.split('/').last().unwrap_or("/")
}
