#![no_std]
#![no_main]
#![feature(format_args_nl)]
#![feature(new_uninit)]

mod dev;
pub mod fd_table;
pub mod pipe;

extern crate alloc;

use alloc::{
    collections::BTreeMap,
    string::{String, ToString},
    sync::Arc,
};

use driver::{println, BLOCK_DEVICE};
use spin::Once;
use sync::mutex::SpinNoIrqLock;
use systype::SysResult;
use vfs_core::{Dentry, DentryMeta, DirEntry, File, FileMeta, FileSystemType, MountFlags};

type Mutex<T> = SpinNoIrqLock<T>;

pub static FS_MANAGER: Mutex<BTreeMap<String, Arc<dyn FileSystemType>>> =
    Mutex::new(BTreeMap::new());

static SYS_ROOT_DENTRY: Once<Arc<dyn Dentry>> = Once::new();

type DiskFsType = fat32::FatFsType;

pub const DISK_FS_NAME: &str = "fat32";

fn register_all_fs() {
    let diskfs = DiskFsType::new();
    FS_MANAGER.lock().insert(diskfs.name_string(), diskfs);

    log::info!("[vfs] register fs success");
}

/// Init the filesystem
pub fn init_filesystem() {
    register_all_fs();
    let diskfs = FS_MANAGER.lock().get(DISK_FS_NAME).unwrap().clone();
    let diskfs_root = diskfs
        .mount(
            "/",
            MountFlags::empty(),
            Some(BLOCK_DEVICE.get().unwrap().clone()),
        )
        .unwrap();
    SYS_ROOT_DENTRY.call_once(|| diskfs_root);
    test().unwrap();
}

pub fn sys_root_dentry() -> Arc<dyn Dentry> {
    SYS_ROOT_DENTRY.get().unwrap().clone()
}

pub fn test() -> SysResult<()> {
    let mut buf = [0; 512];
    let sb = FS_MANAGER
        .lock()
        .get(DISK_FS_NAME)
        .unwrap()
        .get_sb("/")
        .unwrap();

    let root_dentry = sb.root_dentry();

    // let root_dir = root_dentry.open()?;
    // while let Some(dirent) = root_dir.read_dir()? {
    //     println!("{}", dirent.name);
    // }

    // let dentry = root_dentry.lookup("busybox")?;
    // let file = dentry.open()?;
    // file.read(0, &mut buf);
    // log::info!("{}", file.path());
    // log::info!("{:?}", buf);

    Ok(())
}
