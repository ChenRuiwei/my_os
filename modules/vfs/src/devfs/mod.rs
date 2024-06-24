use alloc::sync::Arc;

use driver::BlockDevice;
use systype::SysResult;
use vfs_core::{
    Dentry, FileSystemType, FileSystemTypeMeta, InodeMode, Path, SuperBlock, SuperBlockMeta,
};

use self::{
    tty::{TtyDentry, TtyFile, TtyInode, TTY},
    zero::{ZeroDentry, ZeroFile, ZeroInode},
};
use crate::{
    simplefs::{dentry::SimpleDentry, inode::SimpleInode},
    sys_root_dentry,
};

pub mod stdio;
pub mod tty;
mod zero;

pub fn init_devfs(root_dentry: Arc<dyn Dentry>) -> SysResult<()> {
    let sb = root_dentry.super_block();

    let zero_dentry = ZeroDentry::new("zero", sb.clone(), Some(root_dentry.clone()));
    root_dentry.insert(zero_dentry.clone());
    let zero_inode = ZeroInode::new(sb.clone());
    zero_dentry.set_inode(zero_inode);

    let tty_dentry = TtyDentry::new("tty", sb.clone(), Some(root_dentry.clone()));
    root_dentry.insert(tty_dentry.clone());
    let tty_inode = TtyInode::new(sb.clone());
    tty_dentry.set_inode(tty_inode);
    let tty_file = TtyFile::new(tty_dentry.clone(), tty_dentry.inode()?);
    TTY.call_once(|| tty_file);
    Ok(())
}

pub struct DevFsType {
    meta: FileSystemTypeMeta,
}

impl DevFsType {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            meta: FileSystemTypeMeta::new("devfs"),
        })
    }
}

impl FileSystemType for DevFsType {
    fn meta(&self) -> &FileSystemTypeMeta {
        &self.meta
    }

    fn base_mount(
        self: alloc::sync::Arc<Self>,
        name: &str,
        parent: Option<Arc<dyn Dentry>>,
        _flags: vfs_core::MountFlags,
        dev: Option<alloc::sync::Arc<dyn driver::BlockDevice>>,
    ) -> systype::SysResult<alloc::sync::Arc<dyn vfs_core::Dentry>> {
        let sb = DevSuperBlock::new(dev, self.clone());
        let mount_dentry = SimpleDentry::new(name, sb.clone(), parent.clone());
        let mount_inode = SimpleInode::new(InodeMode::DIR, sb.clone(), 0);
        mount_dentry.set_inode(mount_inode.clone());
        if let Some(parent) = parent {
            parent.insert(mount_dentry.clone());
        }
        self.insert_sb(&mount_dentry.path(), sb);
        Ok(mount_dentry)
    }

    fn kill_sb(&self, _sb: alloc::sync::Arc<dyn vfs_core::SuperBlock>) -> systype::SysResult<()> {
        todo!()
    }
}

struct DevSuperBlock {
    meta: SuperBlockMeta,
}

impl DevSuperBlock {
    pub fn new(
        device: Option<Arc<dyn BlockDevice>>,
        fs_type: Arc<dyn FileSystemType>,
    ) -> Arc<Self> {
        Arc::new(Self {
            meta: SuperBlockMeta::new(device, fs_type),
        })
    }
}

impl SuperBlock for DevSuperBlock {
    fn meta(&self) -> &SuperBlockMeta {
        &self.meta
    }

    fn stat_fs(&self) -> systype::SysResult<vfs_core::StatFs> {
        todo!()
    }

    fn sync_fs(&self, _wait: isize) -> systype::SysResult<()> {
        todo!()
    }
}
