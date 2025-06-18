use std::{
    cell::{Cell, UnsafeCell},
    future::Future,
    marker::PhantomPinned,
    mem::transmute,
    num::NonZeroUsize,
    pin::{Pin, pin},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicPtr, AtomicUsize, Ordering},
    },
    task::{Context, Poll, RawWaker, RawWakerVTable, Waker},
    thread::{self, JoinHandle},
};

use core_affinity::{CoreId, get_core_ids};
use platform::Executor;

thread_local! {
    static THREAD_INDEX: Cell<usize> = const { Cell::new(usize::MAX) };
}

#[derive(Default)]
struct ExecutorState {
    /// If `Some`, a thread should take this task and begin executing it
    task: Option<Pin<&'static Task<'static>>>,
    /// Contains all active iter tasks, which free threads can help with
    active_parallel_iter_tasks: Vec<Option<Pin<&'static NonAsyncIterTask<'static>>>>,
    /// Should threads join?
    join: bool,
}

static EXECUTOR_STATE: Mutex<ExecutorState> = Mutex::new(ExecutorState {
    task: None,
    active_parallel_iter_tasks: Vec::new(),
    join: false,
});

static EXECUTOR_CONDVAR: Condvar = Condvar::new();

#[derive(Default)]
struct BlockingTaskInfo {
    task: AtomicPtr<Task<'static>>,
    cvar: Condvar,
    completed: Mutex<bool>,
}

pub struct TaskExecutor {
    blocking_task_info: Arc<BlockingTaskInfo>,
    thread_join_handles: Vec<JoinHandle<()>>,
}

// Using AtomicU8 for init_count doesn't really make sense because we needed to lock
// the cvar regardless, and converting init_count and then adding an additional Mutex<()>
// just for the locking seems more complicated than just using Mutex<u8>
#[allow(clippy::mutex_integer)]
impl TaskExecutor {
    /// # Safety
    ///
    /// `TaskExecutor` is global. `TaskExecutor::new()` may only be called once.
    pub unsafe fn new() -> Self {
        let thread_count = TaskExecutor::available_parallelism();

        EXECUTOR_STATE
            .lock()
            .unwrap()
            .active_parallel_iter_tasks
            .resize(thread_count.get(), None);

        struct ThreadInfo {
            init_count: Mutex<u8>,
            cvar: Condvar,
            core_ids: Option<Vec<CoreId>>,
        }

        let thread_init = Arc::new(ThreadInfo {
            init_count: Mutex::new(0),
            cvar: Condvar::new(),
            core_ids: get_core_ids(),
        });

        if let Some(core_ids) = &thread_init.core_ids {
            if thread_count.get() > core_ids.len() {
                log::warn!(
                    "thread count ({}) > available thread core ids ({})",
                    core_ids.len(),
                    thread_count.get(),
                );
            }
        }

        let blocking_task_info = Arc::new(BlockingTaskInfo::default());

        let mut thread_join_handles = Vec::with_capacity(thread_count.get());

        for thread_index in 0..thread_count.get() {
            let thread_init = thread_init.clone();

            let blocking_task_info = blocking_task_info.clone();

            thread_join_handles.push(thread::spawn(move || {
                if let Some(id) = thread_init
                    .core_ids
                    .as_ref()
                    .and_then(|ids| ids.get(thread_index))
                {
                    core_affinity::set_for_current(*id);
                }

                THREAD_INDEX.set(thread_index);
                *thread_init.init_count.lock().unwrap() += 1;
                thread_init.cvar.notify_one();
                drop(thread_init);

                loop {
                    // block until thread needs to do something
                    let guard = EXECUTOR_STATE.lock().unwrap();
                    let mut guard = EXECUTOR_CONDVAR
                        .wait_while(guard, |state| {
                            state.task.is_none()
                                && state.active_parallel_iter_tasks.iter().all(Option::is_none)
                                && !state.join
                        })
                        .unwrap();

                    if guard.join {
                        break;
                    }

                    if let Some(task) = guard.task.take() {
                        drop(guard);

                        let ptr = &*task as *const _;

                        let waker =
                            RawWaker::new(((&*task) as *const Task<'_>).cast::<()>(), &VTABLE);
                        let waker = unsafe { Waker::from_raw(waker) };
                        let mut context = Context::from_waker(&waker);

                        match task.poll_future(&mut context) {
                            TaskStatus::Ready => {
                                if ptr == blocking_task_info.task.load(Ordering::Acquire) {
                                    *blocking_task_info.completed.lock().unwrap() = true;
                                    blocking_task_info.cvar.notify_one();
                                }
                            }
                            _ => {
                                unreachable!("pending tasks unsupported");
                            }
                        }
                    } else {
                        // help other threads with their iter task
                        let task = guard
                            .active_parallel_iter_tasks
                            .iter()
                            .find_map(|task| *task)
                            .unwrap();

                        // increment worker count BEFORE dropping the guard, so that the task doesn't deallocate
                        *task.worker_count.lock().unwrap() += 1;

                        drop(guard);

                        task.execute_to_completion();
                    }
                }
            }));
        }

        let _init_guard = thread_init
            .cvar
            .wait_while(thread_init.init_count.lock().unwrap(), |count| {
                (*count as usize) < thread_count.get()
            })
            .unwrap();

        Self {
            blocking_task_info,
            thread_join_handles,
        }
    }
}

impl Executor for TaskExecutor {
    fn available_parallelism() -> NonZeroUsize {
        thread::available_parallelism().expect("unable to determine available parallelism")
    }

    #[inline]
    fn thread_index() -> usize {
        THREAD_INDEX.get()
    }

    fn parallel_iter<F>(len: usize, f: F)
    where
        F: Fn(usize, usize) + Send + Sync,
    {
        if len == 0 {
            return;
        }

        let thread_index = Self::thread_index();

        // special case len == 1, just run immediately and return
        if len == 1 {
            f(0, thread_index);
            return;
        }

        let f = pin!(f);

        let task = pin!(NonAsyncIterTask::new(f, len));
        let task = task.as_ref();

        // SAFETY: we block until the task completes.
        let task = unsafe {
            transmute::<Pin<&NonAsyncIterTask<'_>>, Pin<&'static NonAsyncIterTask<'static>>>(task)
        };

        // notify other threads that we could use help
        EXECUTOR_STATE.lock().unwrap().active_parallel_iter_tasks[thread_index] = Some(task);
        EXECUTOR_CONDVAR.notify_all();

        // start iteration
        loop {
            let i = task.iter_index.fetch_add(1, Ordering::Relaxed);

            if i < len {
                (task.f)(i, thread_index);
            } else {
                // atomically remove self from the executor state
                EXECUTOR_STATE.lock().unwrap().active_parallel_iter_tasks[thread_index] = None;

                // block until other threads are done
                let _guard = task
                    .cvar
                    .wait_while(task.worker_count.lock().unwrap(), |count| *count > 0)
                    .unwrap();

                break;
            }
        }
    }

    fn execute_blocking(&mut self, future: Pin<&mut (dyn Future<Output = ()> + Send)>) {
        let join_handle = pin!(AtomicUsize::default());
        let join_handle = join_handle.as_ref();

        let task = pin!(Task::new(future, join_handle));
        let task = task.as_ref();

        // SAFETY: we block until the task completes.
        let task = unsafe { transmute::<Pin<&Task<'_>>, Pin<&'static Task<'static>>>(task) };

        self.blocking_task_info
            .task
            .store(&*task as *const Task<'_> as *mut _, Ordering::Release);

        // set current task and wake a thread to work on it
        EXECUTOR_STATE.lock().unwrap().task = Some(task);
        EXECUTOR_CONDVAR.notify_one();

        // wait for task to complete
        let mut task_guard = self
            .blocking_task_info
            .cvar
            .wait_while(
                self.blocking_task_info.completed.lock().unwrap(),
                |completed| !*completed,
            )
            .unwrap();

        *task_guard = false;
    }
}

impl Drop for TaskExecutor {
    fn drop(&mut self) {
        EXECUTOR_STATE.lock().unwrap().join = true;
        EXECUTOR_CONDVAR.notify_all();

        for thread in self.thread_join_handles.drain(..) {
            thread.join().unwrap();
        }
    }
}

const VTABLE: RawWakerVTable = RawWakerVTable::new(task_clone, |_| {}, |_| {}, |_| {});

fn task_clone(task: *const ()) -> RawWaker {
    RawWaker::new(task, &VTABLE)
}

enum TaskStatus {
    Ready,
    Pending,
    UnableToPoll,
}

// We require at least an alignment of 2 so that the lower bit of the pointer may act as a flag.
// This allows "done" and the pointer to the parent task to be set in a single atomic operation.
#[repr(align(2))]
struct Task<'a> {
    future: UnsafeCell<Pin<&'a mut (dyn Future<Output = ()> + Send)>>,
    join_handle: Pin<&'a AtomicUsize>,
    executing: AtomicBool,
    // the waker system relies on stable Task addresses
    _pinned: PhantomPinned,
}

// SAFETY: access to the future is protected by the `executing` atomic flag
unsafe impl Sync for Task<'_> {}

impl<'a> Task<'a> {
    fn new(
        future: Pin<&'a mut (dyn Future<Output = ()> + Send)>,
        join_handle: Pin<&'a AtomicUsize>,
    ) -> Self {
        Self {
            future: future.into(),
            join_handle,
            executing: AtomicBool::new(false),
            _pinned: PhantomPinned,
        }
    }

    fn poll_future(&self, context: &mut Context<'_>) -> TaskStatus {
        if self
            .executing
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            let future = unsafe { &mut *self.future.get() };
            if future.as_mut().poll(context).is_ready() {
                let pending_task =
                    self.join_handle.fetch_or(1, Ordering::SeqCst) as *const Task<'_>;
                // LSB is guaranteed to be 0 (task has never completed), so we don't need to mask it off
                if !pending_task.is_null() {
                    unreachable!("pending tasks unsupported");
                }

                TaskStatus::Ready
            } else {
                self.executing.store(false, Ordering::Release);
                TaskStatus::Pending
            }
        } else {
            TaskStatus::UnableToPoll
        }
    }
}

struct JoinHandleTask<'a> {
    join_handle: Pin<&'a AtomicUsize>,
}

impl Future for JoinHandleTask<'_> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let task = cx.waker().data() as usize;
        // We always write the parent task pointer to the join handle. This signals to the child
        // task that the parent wants to be woken on completion, but if the child task is already
        // complete, the join handle will never be read. This allows both the parent task pointer
        // assignment and the LSB/done bit checking to be done in a single atomic operation.
        if self.join_handle.fetch_or(task, Ordering::SeqCst) & 1 == 1 {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}

struct NonAsyncIterTask<'a> {
    f: Pin<&'a (dyn Fn(usize, usize) + Send + Sync)>,
    cvar: Condvar,
    worker_count: Mutex<usize>,
    iter_index: AtomicUsize,
    len: usize,
    _pinned: PhantomPinned,
}

impl<'a> NonAsyncIterTask<'a> {
    fn new(f: Pin<&'a (dyn Fn(usize, usize) + Send + Sync)>, len: usize) -> Self {
        Self {
            f,
            cvar: Condvar::new(),
            worker_count: Mutex::new(0),
            iter_index: AtomicUsize::new(0),
            len,
            _pinned: PhantomPinned,
        }
    }

    fn execute_to_completion(&self) {
        let thread_index = TaskExecutor::thread_index();

        loop {
            let i = self.iter_index.fetch_add(1, Ordering::Relaxed);

            if i < self.len {
                (self.f)(i, thread_index);
            } else {
                // decrement worker count
                let mut worker_count = self.worker_count.lock().unwrap();
                *worker_count -= 1;

                // if we're the last worker, notify the waiting thread to wake up.
                // don't drop the guard first, because the cvar could be deallocated!
                if *worker_count == 0 {
                    self.cvar.notify_one();
                }

                break;
            }
        }
    }
}
