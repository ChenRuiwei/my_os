//! Implementation of syscalls

mod consts;
mod fs;
pub mod futex;
mod misc;
mod mm;
mod process;
mod resource;
mod sched;
mod signal;
mod time;

use ::futex::RobustListHead;
pub use consts::SyscallNo;
use consts::*;
pub use fs::resolve_path;
use fs::*;
use misc::*;
pub use mm::MmapFlags;
use mm::*;
pub use process::CloneFlags;
use process::*;
use resource::*;
use signal::*;
use systype::SyscallResult;
use time::*;

use crate::{
    mm::{FutexWord, UserReadPtr, UserWritePtr},
    syscall::{
        futex::{sys_futex, sys_get_robust_list, sys_set_robust_list},
        sched::*,
    },
};

#[cfg(feature = "strace")]
pub const STRACE_COLOR_CODE: u8 = 35; // Purple

/// Syscall trace.
// TODO: syscall trace with exact args and return value
#[cfg(feature = "strace")]
#[macro_export]
macro_rules! strace {
    ($fmt:expr, $($args:tt)*) => {
        use $crate::{
            processor::hart::{local_hart, current_task}
        };
        $crate::impls::print_in_color(
            format_args!(concat!("[SYSCALL][H{},P{},T{}] ",  $fmt," \n"),
            local_hart().hart_id(),
            current_task().pid(),
            current_task().tid(),
            $($args)*),
            $crate::syscall::STRACE_COLOR_CODE
        );
    }
}
#[cfg(not(feature = "strace"))]
#[macro_export]
macro_rules! strace {
    ($fmt:literal $(, $($arg:tt)+)?) => {};
}

/// Handle syscall exception with `syscall_id` and other arguments.
pub async fn syscall(syscall_no: usize, args: [usize; 6]) -> usize {
    use SyscallNo::*;

    let Some(syscall_no) = SyscallNo::from_repr(syscall_no) else {
        log::error!("Syscall number not included: {}", syscall_no);
        unimplemented!()
    };
    log::info!("[syscall] handle {syscall_no}");
    strace!("{}, args: {:?}", syscall_no, args);
    let result = match syscall_no {
        // Process
        EXIT => sys_exit(args[0] as _),
        EXIT_GROUP => sys_exit_group(args[0] as _),
        EXECVE => sys_execve(args[0].into(), args[1].into(), args[2].into()).await,
        SCHED_YIELD => sys_sched_yield().await,
        CLONE => sys_clone(args[0], args[1], args[2], args[3], args[4]),
        WAIT4 => sys_wait4(args[0] as _, args[1].into(), args[2] as _, args[3]).await,
        GETTID => sys_gettid(),
        GETPID => sys_getpid(),
        GETPPID => sys_getppid(),
        GETPGID => sys_getpgid(args[0]),
        SET_TID_ADDRESS => sys_set_tid_address(args[0]),
        GETUID => sys_getuid(),
        GETEUID => sys_geteuid(),
        SETPGID => sys_setpgid(args[0], args[1]),
        // Memory
        BRK => sys_brk(args[0].into()),
        MMAP => sys_mmap(
            args[0].into(),
            args[1],
            args[2] as _,
            args[3] as _,
            args[4],
            args[5],
        ),
        MUNMAP => sys_munmap(args[0].into(), args[1]),
        // File system
        READ => sys_read(args[0], args[1].into(), args[2]).await,
        WRITE => sys_write(args[0], args[1].into(), args[2]).await,
        OPENAT => sys_openat(args[0] as _, args[1].into(), args[2] as _, args[3] as _),
        CLOSE => sys_close(args[0]),
        MKDIR => sys_mkdirat(args[0] as _, args[1].into(), args[2] as _),
        GETCWD => sys_getcwd(args[0].into(), args[1]),
        CHDIR => sys_chdir(args[0].into()),
        DUP => sys_dup(args[0]),
        DUP3 => sys_dup3(args[0], args[1], args[2] as _),
        FSTAT => sys_fstat(args[0], args[1].into()),
        FSTATAT => sys_fstatat(args[0] as _, args[1].into(), args[2].into(), args[3] as _),
        GETDENTS64 => sys_getdents64(args[0], args[1], args[2]),
        UNLINKAT => sys_unlinkat(args[0] as _, args[1].into(), args[2] as _),
        MOUNT => {
            sys_mount(
                args[0].into(),
                args[1].into(),
                args[2].into(),
                args[3] as _,
                args[4].into(),
            )
            .await
        }
        UMOUNT2 => sys_umount2(args[0].into(), args[1] as _).await,
        PIPE2 => sys_pipe2(args[0].into(), args[1] as _),
        IOCTL => sys_ioctl(args[0], args[1], args[2]),
        FCNTL => sys_fcntl(args[0], args[1] as _, args[2]),
        WRITEV => sys_writev(args[0], args[1].into(), args[2]).await,
        READV => sys_readv(args[0], args[1].into(), args[2]).await,
        PPOLL => sys_ppoll(args[0].into(), args[1], args[2].into(), args[3]).await,
        SENDFILE => sys_sendfile(args[0], args[1], args[2].into(), args[3]).await,
        // Signal
        RT_SIGPROCMASK => sys_rt_sigprocmask(args[0], args[1].into(), args[2].into()),
        RT_SIGACTION => sys_rt_sigaction(args[0] as _, args[1].into(), args[2].into()),
        KILL => sys_kill(args[0] as _, args[1] as _),
        TKILL => sys_tkill(args[0] as _, args[1] as _),
        TGKILL => sys_tgkill(args[0] as _, args[1] as _, args[2] as _),
        RT_SIGRETURN => sys_rt_sigreturn(),
        RT_SIGSUSPEND => sys_rt_sigsuspend(args[0].into()).await,
        // times
        GETTIMEOFDAY => sys_gettimeofday(args[0].into(), args[1]),
        TIMES => sys_times(args[0].into()),
        NANOSLEEP => sys_nanosleep(args[0].into(), args[1].into()).await,
        CLOCK_GETTIME => sys_clock_gettime(args[0], args[1].into()),
        CLOCK_SETTIME => sys_clock_settime(args[0], args[1].into()),
        CLOCK_GETRES => sys_clock_getres(args[0], args[1].into()),
        GETITIMER => sys_getitimer(args[0] as _, args[1].into()),
        SETITIMER => sys_setitimer(args[0] as _, args[1].into(), args[2].into()),
        // Futex
        FUTEX => {
            sys_futex(
                args[0].into(),
                args[1] as _,
                args[2] as _,
                args[3] as _,
                args[4] as _,
                args[5] as _,
            )
            .await
        }
        GET_ROBUST_LIST => sys_get_robust_list(args[0] as _, args[1].into(), args[2].into()),
        SET_ROBUST_LIST => sys_set_robust_list(args[0].into(), args[1]),
        // Schedule
        SCHED_SETSCHEDULER => sys_sched_setscheduler(),
        SCHED_GETSCHEDULER => sys_sched_getscheduler(),
        SCHED_GETPARAM => sys_sched_getparam(),
        SCHED_SETAFFINITY => sys_sched_setaffinity(args[0], args[1], args[2].into()),
        SCHED_GETAFFINITY => sys_sched_getaffinity(args[0], args[1], args[2].into()),
        // Miscellaneous
        UNAME => sys_uname(args[0].into()),
        GETRUSAGE => sys_getrusage(args[0] as _, args[1].into()),
        SYSLOG => sys_syslog(args[0], args[1].into(), args[2]),
        _ => {
            log::error!("Unsupported syscall: {}", syscall_no);
            Ok(0)
        }
    };
    match result {
        Ok(ret) => {
            log::info!("[syscall] {syscall_no} return val {ret:#x}");
            ret
        }
        Err(e) => {
            log::warn!("[syscall] {syscall_no} return err {e:?}",);
            -(e as isize) as usize
        }
    }
}
