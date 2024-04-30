use alloc::sync::Arc;
use core::any::TypeId;

use vfs_core::{Inode, InodeMeta, InodeMode, InodeType, Stat, SuperBlock};

use crate::{FatFile, Mutex, Shared};

pub struct FatFileInode {
    meta: InodeMeta,
    pub file: Shared<FatFile>,
}

impl FatFileInode {
    pub fn new(super_block: Arc<dyn SuperBlock>, file: FatFile) -> Arc<Self> {
        let size = file.size().unwrap().try_into().unwrap();
        let inode = Arc::new(Self {
            meta: InodeMeta::new(
                InodeMode::from_type(InodeType::File),
                super_block.clone(),
                size,
            ),
            file: Arc::new(Mutex::new(file)),
        });
        super_block.push_inode(inode.clone());
        inode
    }
}

impl Inode for FatFileInode {
    fn meta(&self) -> &InodeMeta {
        &self.meta
    }

    fn get_attr(&self) -> systype::SysResult<Stat> {
        let meta_inner = self.meta.inner.lock();
        let mode = self.meta.mode.bits();
        let len = meta_inner.size;
        Ok(Stat {
            st_dev: 0,
            st_ino: self.meta.ino as u64,
            st_mode: mode,
            st_nlink: 1,
            st_uid: 0,
            st_gid: 0,
            st_rdev: 0,
            __pad: 0,
            st_size: len as u64,
            st_blksize: 512,
            __pad2: 0,
            st_blocks: (len / 512) as u64,
            st_atime: meta_inner.atime,
            st_mtime: meta_inner.mtime,
            st_ctime: meta_inner.ctime,
            unused: 0,
        })
    }
}
