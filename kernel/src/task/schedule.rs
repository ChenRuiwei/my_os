use alloc::sync::Arc;
use core::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use super::Task;
use crate::{
    processor::{self, ctx::EnvContext, current_task},
    trap,
};

struct YieldFuture {
    pub has_yielded: bool,
}

impl YieldFuture {
    const fn new() -> Self {
        Self { has_yielded: false }
    }
}

impl Future for YieldFuture {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        match self.has_yielded {
            true => Poll::Ready(()),
            false => {
                self.has_yielded = true;
                cx.waker().wake_by_ref();
                Poll::Pending
            }
        }
    }
}

/// The outermost future for user task, i.e. the future that wraps one thread's
/// task future (doing some env context changes e.g. pagetable switching)
pub struct UserTaskFuture<F: Future + Send + 'static> {
    task: Arc<Task>,
    env: EnvContext,
    future: F,
}

impl<F: Future + Send + 'static> UserTaskFuture<F> {
    #[inline]
    pub fn new(task: Arc<Task>, future: F) -> Self {
        Self {
            task,
            env: EnvContext::new(),
            future,
        }
    }
}

impl<F: Future + Send + 'static> Future for UserTaskFuture<F> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = unsafe { self.get_unchecked_mut() };
        let hart = processor::local_hart();
        hart.enter_user_task_switch(&mut this.task, &mut this.env);
        let ret = unsafe { Pin::new_unchecked(&mut this.future).poll(cx) };
        hart.leave_user_task_switch(&mut this.env);
        ret
    }
}

pub struct KernelTaskFuture<F: Future<Output = ()> + Send + 'static> {
    env: EnvContext,
    future: F,
}

impl<F: Future<Output = ()> + Send + 'static> KernelTaskFuture<F> {
    pub fn new(future: F) -> Self {
        Self {
            env: EnvContext::new(),
            future,
        }
    }
}

impl<F: Future<Output = ()> + Send + 'static> Future for KernelTaskFuture<F> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = unsafe { self.get_unchecked_mut() };
        let hart = processor::local_hart();
        hart.kernel_task_switch(&mut this.env);
        let ret = unsafe { Pin::new_unchecked(&mut this.future).poll(cx) };
        hart.kernel_task_switch(&mut this.env);
        ret
    }
}

pub async fn task_loop(task: Arc<Task>) {
    task.set_waker(async_utils::take_waker().await);
    loop {
        trap::user_trap::trap_return();

        // next time when user traps into kernel, it will come back here
        trap::user_trap::trap_handler().await;

        if task.is_zombie() {
            log::debug!("thread {} terminated", current_task().pid());
            break;
        }
    }

    handle_exit(task);
}

pub fn handle_exit(task: Arc<Task>) {
    panic!()
}

/// Spawn a new async user task
pub fn spawn_user_task(user_task: Arc<Task>) {
    let future = UserTaskFuture::new(user_task.clone(), task_loop(user_task));
    let (runnable, task) = executor::spawn(future);
    runnable.schedule();
    task.detach();
}

/// Spawn a new async kernel task (used for doing some kernel init work or timed
/// tasks)
pub fn spawn_kernel_task<F: Future<Output = ()> + Send + 'static>(kernel_task: F) {
    let future = KernelTaskFuture::new(kernel_task);
    let (runnable, task) = executor::spawn(future);
    runnable.schedule();
    task.detach();
}
