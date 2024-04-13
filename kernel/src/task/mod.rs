pub mod aux;
mod manager;
mod schedule;
pub mod signal;
pub mod task;
mod tid;

pub use manager::TASK_MANAGER;
pub use schedule::{spawn_kernel_task, spawn_user_task, yield_now};
pub use task::Task;
pub use tid::{PGid, Pid, Tid};

use crate::{loader::get_app_data_by_name, mm::memory_space};

pub fn add_init_proc() {
    let elf_data = get_app_data_by_name("exec_test").unwrap();

    Task::spawn_from_elf(elf_data);
}
