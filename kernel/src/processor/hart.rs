use alloc::{boxed::Box, sync::Arc};
use core::arch::asm;

use arch::interrupts::{disable_interrupt, enable_interrupt};
use config::processor::HART_NUM;
use riscv::register::sstatus::{self, FS};
use spin::Once;
use sync::cell::SyncUnsafeCell;

use super::ctx::EnvContext;
use crate::{
    mm::{self, PageTable},
    stack_trace,
    task::Task,
};

const HART_EACH: Hart = Hart::new();
pub static mut HARTS: [Hart; HART_NUM] = [HART_EACH; HART_NUM];

/// The processor has several `Hart`s
pub struct Hart {
    hart_id: usize,
    task: Option<Arc<Task>>,
    env: EnvContext,
}

impl Hart {
    pub fn env(&self) -> &EnvContext {
        &self.env
    }
    pub fn env_mut(&mut self) -> &mut EnvContext {
        &mut self.env
    }
    pub fn current_task(&self) -> &Arc<Task> {
        stack_trace!();
        self.task
    }
}

impl Hart {
    pub const fn new() -> Self {
        Hart {
            hart_id: 0,
            task: None,
            env: EnvContext::new(),
        }
    }
    pub fn set_hart_id(&mut self, hart_id: usize) {
        self.hart_id = hart_id;
    }
    pub fn hart_id(&self) -> usize {
        self.hart_id
    }

    /// Change thread(task) context,
    /// Now only change page table temporarily
    pub fn enter_user_task_switch(&mut self, task: &mut Arc<Task>, env: &mut EnvContext) {
        // self can only be an executor running
        disable_interrupt();
        let old_env = self.env();
        let sie = EnvContext::env_change(env, old_env);
        self.task = Some(task);
        core::mem::swap(self.env_mut(), env);
        if sie {
            enable_interrupt();
        }
    }
    pub fn leave_user_task_switch(&mut self, task: &mut Arc<Task>, env: &mut EnvContext) {
        disable_interrupt();
        let old_env = self.env();
        let sie = EnvContext::env_change(env, old_env);
        mm::activate_kernel_space();
        self.task = None;
        core::mem::swap(self.env_mut(), env);
        if sie {
            enable_interrupt();
        }
    }
    pub fn kernel_task_switch(&mut self, env: &mut EnvContext) {
        disable_interrupt();
        let old_env = self.env();
        let sie = EnvContext::env_change(env, old_env);
        core::mem::swap(self.env_mut(), env);
        if sie {
            enable_interrupt();
        }
    }
}

unsafe fn get_hart_by_id(hart_id: usize) -> &'static mut Hart {
    &mut HARTS[hart_id]
}

/// Set the cpu hart control block according to `hard_id`
/// set register tp points to hart control block
pub fn set_local_hart(hart_id: usize) {
    unsafe {
        let hart = get_hart_by_id(hart_id);
        hart.set_hart_id(hart_id);
        let hart_addr = hart as *const _ as usize;
        asm!("mv tp, {}", in(reg) hart_addr);
    }
}

/// Get the current `Hart` by `tp` register.
pub fn local_hart() -> &'static mut Hart {
    unsafe {
        let tp: usize;
        asm!("mv {}, tp", out(reg) tp);
        &mut *(tp as *mut Hart)
    }
}

pub fn init(hart_id: usize) {
    println!("start to init hart {}...", hart_id);
    set_local_hart(hart_id);
    unsafe {
        sstatus::set_fs(FS::Initial);
    }
    println!("init hart {} finished", hart_id);
}
