use alloc::{
    collections::BTreeMap,
    string::String,
    sync::{Arc, Weak},
    vec::Vec,
};
use core::{
    cell::SyncUnsafeCell,
    sync::atomic::{AtomicI32, AtomicUsize, Ordering},
    task::Waker,
};

use arch::memory::sfence_vma_all;
use config::{mm::USER_STACK_SIZE, process::INIT_PROC_PID};
use memory::VirtAddr;
use signal::{
    action::{SigHandlers, SigPending},
    signal_stack::SignalStack,
    sigset::{Sig, SigSet},
};
use sync::mutex::SpinNoIrqLock;
use time::stat::TaskTimeStat;

use super::tid::{Pid, Tid, TidHandle};
use crate::{
    mm::MemorySpace,
    syscall,
    task::{manager::TASK_MANAGER, schedule, tid::alloc_tid},
    trap::TrapContext,
};

type Shared<T> = Arc<SpinNoIrqLock<T>>;

fn new_shared<T>(data: T) -> Shared<T> {
    Arc::new(SpinNoIrqLock::new(data))
}

/// User task control block, a.k.a. process control block.
///
/// We treat processes and threads as tasks, consistent with the approach
/// adopted by Linux. A process is a task that is the leader of a `ThreadGroup`.
pub struct Task {
    // Immutable
    /// Tid of the task.
    tid: TidHandle,
    /// Whether the task is the leader.
    is_leader: bool,

    // Mutable
    /// Whether this task is a zombie. Locked because of other task may operate
    /// this state, e.g. execve will kill other tasks.
    state: SpinNoIrqLock<TaskState>,
    /// The process's address space
    memory_space: Shared<MemorySpace>,
    /// Parent process
    parent: Shared<Option<Weak<Task>>>,
    /// Children processes
    // NOTE: Arc<Task> can only be hold by `Hart`, `UserTaskFuture` and parent `Task`. Unused task
    // will be automatically dropped by previous two structs. However, it should be treated with
    // great care to drop task in `children`.
    children: Shared<BTreeMap<Tid, Arc<Task>>>,
    /// Exit code of the current process
    exit_code: AtomicI32,
    ///
    trap_context: SyncUnsafeCell<TrapContext>,
    ///
    waker: SyncUnsafeCell<Option<Waker>>,
    ///
    thread_group: Shared<ThreadGroup>,
    /// received signals
    sig_pending: SpinNoIrqLock<SigPending>,
    /// 存储了对每个信号的处理方法。
    sig_handlers: SyncUnsafeCell<SigHandlers>,
    /// 信号掩码用于标识哪些信号被阻塞，不应该被该进程处理。
    /// 这是进程级别的持续性设置，通常用于防止进程在关键操作期间被中断.
    /// 注意与信号处理时期的临时掩码做区别
    sig_mask: SyncUnsafeCell<SigSet>,
    /// User can set `sig_stack` by `sys_signalstack`.
    sig_stack: SyncUnsafeCell<Option<SignalStack>>,
    sig_ucontext_ptr: AtomicUsize,
    time_stat: SyncUnsafeCell<TaskTimeStat>,
}

impl core::fmt::Debug for Task {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Task").field("tid", &self.tid()).finish()
    }
}

impl Drop for Task {
    fn drop(&mut self) {
        log::info!("task {} died!", self.tid());
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum TaskState {
    Running,
    Zombie,
}

macro_rules! with_ {
    ($name:ident, $ty:ty) => {
        paste::paste! {
            pub fn [<with_ $name>]<T>(&self, f: impl FnOnce(&$ty) -> T) -> T {
                f(& self.$name.lock())
            }
            pub fn [<with_mut_ $name>]<T>(&self, f: impl FnOnce(&mut $ty) -> T) -> T {
                f(&mut self.$name.lock())
            }
        }
    };
}

impl Task {
    // TODO: this function is not clear, may be replaced with exec
    pub fn spawn_from_elf(elf_data: &[u8]) {
        let (memory_space, user_sp_top, entry_point, _auxv) = MemorySpace::from_elf(elf_data);

        let trap_context = TrapContext::new(entry_point, user_sp_top);
        let task = Arc::new(Self {
            tid: alloc_tid(),
            is_leader: true,
            state: SpinNoIrqLock::new(TaskState::Running),
            parent: new_shared(None),
            children: new_shared(BTreeMap::new()),
            exit_code: AtomicI32::new(0),
            trap_context: SyncUnsafeCell::new(trap_context),
            memory_space: new_shared(memory_space),
            waker: SyncUnsafeCell::new(None),
            thread_group: new_shared(ThreadGroup::new()),
            sig_pending: SpinNoIrqLock::new(SigPending::new()),
            sig_mask: SyncUnsafeCell::new(SigSet::empty()),
            sig_handlers: SyncUnsafeCell::new(SigHandlers::new()),
            sig_stack: SyncUnsafeCell::new(None),
            time_stat: SyncUnsafeCell::new(TaskTimeStat::new()),
            sig_ucontext_ptr: AtomicUsize::new(0),
        });

        task.thread_group.lock().push_leader(task.clone());

        TASK_MANAGER.add(&task);
        log::debug!("create a new process, pid {}", task.tid());
        schedule::spawn_user_task(task);
    }

    pub fn parent(&self) -> Option<Weak<Self>> {
        self.parent.lock().clone()
    }

    pub fn children(&self) -> BTreeMap<Tid, Arc<Self>> {
        self.children.lock().clone()
    }

    fn state(&self) -> TaskState {
        *self.state.lock()
    }

    pub fn add_child(&self, child: Arc<Task>) {
        self.children
            .lock()
            .try_insert(child.tid(), child)
            .expect("try add child with a duplicate tid");
    }

    pub fn remove_child(&self, tid: Tid) {
        self.children.lock().remove(&tid);
    }

    /// the task is a process or a thread
    pub fn is_leader(&self) -> bool {
        self.is_leader
    }

    /// Pid means tgid.
    pub fn pid(&self) -> Pid {
        self.thread_group.lock().tgid()
    }

    pub fn tid(&self) -> Tid {
        self.tid.0
    }

    pub fn ppid(&self) -> Pid {
        self.parent()
            .expect("Call ppid without a parent")
            .upgrade()
            .unwrap()
            .pid()
    }

    pub fn exit_code(&self) -> i32 {
        self.exit_code.load(Ordering::Relaxed)
    }

    pub fn set_exit_code(&self, exit_code: i32) {
        self.exit_code.store(exit_code, Ordering::Relaxed);
    }

    /// Get the mutable ref of `TrapContext`.
    pub fn trap_context_mut(&self) -> &mut TrapContext {
        unsafe { &mut *self.trap_context.get() }
    }

    /// Set waker for this thread
    pub fn set_waker(&self, waker: Waker) {
        unsafe {
            (*self.waker.get()) = Some(waker);
        }
    }

    pub fn set_zombie(&self) {
        *self.state.lock() = TaskState::Zombie
    }

    pub fn is_zombie(&self) -> bool {
        *self.state.lock() == TaskState::Zombie
    }

    pub fn sig_handlers(&self) -> &mut SigHandlers {
        unsafe { &mut *self.sig_handlers.get() }
    }

    pub fn sig_mask(&self) -> &mut SigSet {
        unsafe { &mut *self.sig_mask.get() }
    }

    /// important: new mask can't block SIGKILL or SIGSTOP
    pub fn sig_mask_replace(&self, new: &mut SigSet) -> SigSet {
        new.remove(SigSet::SIGSTOP | SigSet::SIGKILL);
        let old = unsafe { *self.sig_mask.get() };
        unsafe { *self.sig_mask.get() = *new };
        old
    }

    pub fn signal_stack(&self) -> &mut Option<SignalStack> {
        unsafe { &mut *self.sig_stack.get() }
    }

    pub fn set_signal_stack(&self, stack: Option<SignalStack>) {
        unsafe {
            *self.sig_stack.get() = stack;
        }
    }

    pub fn sig_ucontext_ptr(&self) -> usize {
        self.sig_ucontext_ptr.load(Ordering::Relaxed)
    }

    pub fn set_sig_ucontext_ptr(&self, ptr: usize) {
        self.sig_ucontext_ptr.store(ptr, Ordering::Relaxed)
    }

    pub fn time_stat(&self) -> &mut TaskTimeStat {
        unsafe { &mut *self.time_stat.get() }
    }

    pub unsafe fn switch_page_table(&self) {
        self.memory_space.lock().switch_page_table()
    }

    // TODO:
    pub fn do_clone(
        self: &Arc<Self>,
        flags: syscall::CloneFlags,
        user_stack_begin: Option<VirtAddr>,
    ) -> Arc<Self> {
        use syscall::CloneFlags;
        let tid = alloc_tid();

        let trap_context = SyncUnsafeCell::new(*self.trap_context_mut());
        let state = SpinNoIrqLock::new(self.state());
        let _exit_code = AtomicI32::new(self.exit_code());

        let is_leader;
        let parent;
        let children;
        let thread_group;

        if flags.contains(CloneFlags::THREAD) {
            is_leader = false;
            parent = self.parent.clone();
            children = self.children.clone();
            // will add the new task into the group later
            thread_group = self.thread_group.clone();
        } else {
            is_leader = true;
            parent = new_shared(Some(Arc::downgrade(self)));
            children = new_shared(BTreeMap::new());
            thread_group = new_shared(ThreadGroup::new());
        }

        let memory_space;
        if flags.contains(CloneFlags::VM) {
            memory_space = self.memory_space.clone();
        } else {
            debug_assert!(user_stack_begin.is_none());
            memory_space =
                new_shared(self.with_mut_memory_space(|m| MemorySpace::from_user_lazily(m)));
            // TODO: avoid flushing global entries like kernel mappings
            unsafe { sfence_vma_all() };
        }

        let new = Arc::new(Self {
            tid,
            is_leader,
            state,
            parent,
            children,
            exit_code: AtomicI32::new(0),
            trap_context,
            memory_space,
            waker: SyncUnsafeCell::new(None),
            thread_group,
            sig_pending: SpinNoIrqLock::new(SigPending::new()),
            sig_mask: SyncUnsafeCell::new(SigSet::empty()),
            sig_handlers: SyncUnsafeCell::new(SigHandlers::new()),
            sig_stack: SyncUnsafeCell::new(None),
            time_stat: SyncUnsafeCell::new(TaskTimeStat::new()),
            sig_ucontext_ptr: AtomicUsize::new(0),
        });

        if flags.contains(CloneFlags::THREAD) {
            new.with_mut_thread_group(|tg| tg.push(new.clone()));
        } else {
            new.with_mut_thread_group(|g| g.push_leader(new.clone()));
            self.add_child(new.clone());
        }

        TASK_MANAGER.add(&new);
        new
    }

    // TODO:
    pub fn do_execve(&self, elf_data: &[u8], _argv: Vec<String>, _envp: Vec<String>) {
        log::debug!("[Task::do_execve] parsing elf");
        let mut memory_space = MemorySpace::new_user();
        let (entry, _auxv) = memory_space.parse_and_map_elf(elf_data);

        // NOTE: should do termination before switching page table, so that other
        // threads will trap in by page fault but be terminated before handling
        log::debug!("[Task::do_execve] terminating all threads except the leader");
        self.with_thread_group(|tg| {
            for t in tg.iter() {
                if !t.is_leader() {
                    t.set_zombie();
                }
            }
        });

        log::debug!("[Task::do_execve] changing memory space");
        // NOTE: need to switch to new page table first before dropping old page table,
        // otherwise, there will be a vacuum period without page table which will cause
        // random errors in smp situation
        unsafe { memory_space.switch_page_table() };
        self.with_mut_memory_space(|m| *m = memory_space);

        // alloc stack, and push argv, envp and auxv
        log::debug!("[Task::do_execve] allocing stack");
        let stack_begin = self.with_mut_memory_space(|m| m.alloc_stack(USER_STACK_SIZE));

        // alloc heap
        self.with_mut_memory_space(|m| m.alloc_heap_lazily());

        // init trap context
        self.trap_context_mut()
            .init_user(stack_begin.into(), entry, 0, 0, 0);
    }

    // NOTE: After all of the threads in a thread group is terminated, the parent
    // process of the thread group is sent a SIGCHLD (or other termination) signal.
    // TODO:
    pub fn do_exit(self: &Arc<Self>) {
        log::info!("thread {} do exit", self.tid());
        assert_ne!(
            self.tid(),
            INIT_PROC_PID,
            "initproc die!!!, sepc {:#x}",
            self.trap_context_mut().sepc
        );

        // TODO: send SIGCHLD to parent if this is the leader
        if self.is_leader() {
            if let Some(parent) = self.parent() {
                let _parent = parent.upgrade().unwrap();
            }
        }

        log::debug!("[Task::do_exit] set children to be zombie and reparent them to init");
        debug_assert_ne!(self.tid(), INIT_PROC_PID);
        self.with_mut_children(|children| {
            if children.is_empty() {
                return;
            }
            let init_proc = TASK_MANAGER.init_proc();
            children.values().for_each(|c| {
                c.set_zombie();
                *c.parent.lock() = Some(Arc::downgrade(&init_proc));
            });
            init_proc.children.lock().extend(children.clone());
        });

        // release all fd

        // NOTE: leader will be removed by parent calling `sys_wait4`
        if !self.is_leader() {
            self.with_mut_thread_group(|tg| tg.remove(self));
            TASK_MANAGER.remove(self)
        }
    }

    with_!(children, BTreeMap<Tid, Arc<Task>>);
    with_!(memory_space, MemorySpace);
    with_!(thread_group, ThreadGroup);
    with_!(sig_pending, SigPending);
}

/// Hold a group of threads which belongs to the same process.
// PERF: move leader out to decrease lock granularity
pub struct ThreadGroup {
    members: BTreeMap<Tid, Weak<Task>>,
    leader: Option<Weak<Task>>,
}

impl ThreadGroup {
    pub fn new() -> Self {
        Self {
            members: BTreeMap::new(),
            leader: None,
        }
    }

    pub fn push_leader(&mut self, leader: Arc<Task>) {
        debug_assert!(self.leader.is_none());
        debug_assert!(self.members.is_empty());
        self.leader = Some(Arc::downgrade(&leader));
        self.members.insert(leader.tid(), Arc::downgrade(&leader));
    }

    pub fn push(&mut self, task: Arc<Task>) {
        debug_assert!(self.leader.is_some());
        self.members.insert(task.tid(), Arc::downgrade(&task));
    }

    pub fn remove(&mut self, thread: &Task) {
        debug_assert!(self.leader.is_some());
        self.members.remove(&thread.tid());
    }

    pub fn tgid(&self) -> Tid {
        self.leader.as_ref().unwrap().upgrade().unwrap().tid()
    }

    pub fn iter(&self) -> impl Iterator<Item = Arc<Task>> + '_ {
        self.members.values().map(|t| t.upgrade().unwrap())
    }
}
