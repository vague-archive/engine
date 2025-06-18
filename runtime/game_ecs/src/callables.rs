use std::{
    collections::HashMap,
    mem::MaybeUninit,
    sync::{
        Mutex,
        atomic::{AtomicU32, Ordering},
    },
};

use platform::PlatformLibraryFn;
use void_public::{ComponentId, callable::TaskId};

#[cfg(feature = "state_snapshots")]
mod serialize;

#[cfg_attr(feature = "state_snapshots", derive(snapshot::Deserialize))]
#[cfg_attr(feature = "state_snapshots", derive(snapshot::Serialize))]
struct InFlightTask {
    async_completion_id: ComponentId,
    user_data: Box<[MaybeUninit<u8>]>,
}

/// The struct returned to `Completion<AsyncCompletion>`.
#[cfg_attr(feature = "state_snapshots", derive(snapshot::Deserialize))]
#[cfg_attr(feature = "state_snapshots", derive(snapshot::Serialize))]
pub struct CompletedTask {
    pub return_value: Box<[MaybeUninit<u8>]>,
    pub user_data: Box<[MaybeUninit<u8>]>,
}

#[derive(Default)]
pub struct Callables {
    platform_functions: HashMap<ComponentId, PlatformFunction>,

    /// Stores calls to non-sync functions, which will be dispatched at the end of a frame.
    call_queue: Mutex<Vec<QueuedCall>>,

    /// Stores user data for tasks which are not yet complete.
    in_flight_tasks: Mutex<HashMap<TaskId, InFlightTask>>,

    /// Stores completed tasks via a lookup of an `AsyncCompletion` type.
    completions: HashMap<ComponentId, Vec<CompletedTask>>,

    /// Using an atomic here does not break determinism, because its value is only ever passed to the
    /// platform library whose return value is assumed to be non-deterministic anyways.
    next_task_id: AtomicU32,
}
struct PlatformFunction {
    function: Box<dyn PlatformLibraryFn>,
    is_sync: bool,
}

/// A queued call to a non-sync function.
#[cfg_attr(feature = "state_snapshots", derive(snapshot::Deserialize))]
#[cfg_attr(feature = "state_snapshots", derive(snapshot::Serialize))]
struct QueuedCall {
    function_id: ComponentId,
    parameter_data: Box<[MaybeUninit<u8>]>,
    completion_info: Option<QueuedCompletionInfo>,
}

/// Additional info for queued non-sync function calls with async returns.
#[cfg_attr(feature = "state_snapshots", derive(snapshot::Deserialize))]
#[cfg_attr(feature = "state_snapshots", derive(snapshot::Serialize))]
struct QueuedCompletionInfo {
    async_completion_id: ComponentId,
    user_data: Box<[MaybeUninit<u8>]>,
}

impl Callables {
    pub fn add_platform_function(
        &mut self,
        function_id: ComponentId,
        function: Box<dyn PlatformLibraryFn>,
        is_sync: bool,
    ) {
        self.platform_functions
            .insert(function_id, PlatformFunction { function, is_sync });
    }

    /// Marks a task as completed and adds the task to this frame's completions.
    ///
    /// # Safety
    ///
    /// `return_value` must be valid data which matches the expected type associated with `TaskId`.
    pub unsafe fn complete_task(&mut self, task_id: TaskId, return_value: Box<[MaybeUninit<u8>]>) {
        let Some(task_info) = self.in_flight_tasks.get_mut().unwrap().remove(&task_id) else {
            // no in-flight task data found for completed TaskId
            // this is not a warning, because it's possible for users to ignore return values
            return;
        };

        let completed = CompletedTask {
            return_value,
            user_data: task_info.user_data,
        };

        self.completions
            .entry(task_info.async_completion_id)
            .or_default()
            .push(completed);
    }

    /// Dispatch enqueued calls and clear all completions. Typically called at the end of a frame.
    pub fn clear_call_queue_and_completions(&mut self) {
        for call in self.call_queue.get_mut().unwrap().drain(..) {
            let task_id = *self.next_task_id.get_mut();
            *self.next_task_id.get_mut() += 1;

            if let Some(platform_function) = self.platform_functions.get(&call.function_id) {
                platform_function
                    .function
                    .call(task_id, &call.parameter_data);
            }

            if let Some(completion_info) = call.completion_info {
                let task_info = InFlightTask {
                    async_completion_id: completion_info.async_completion_id,
                    user_data: completion_info.user_data,
                };

                self.in_flight_tasks
                    .get_mut()
                    .unwrap()
                    .insert(task_id, task_info);
            }
        }

        for completions in self.completions.values_mut() {
            completions.clear();
        }
    }

    pub fn call(&self, function_id: ComponentId, parameter_data: &[MaybeUninit<u8>]) {
        if let Some(function) = self.platform_functions.get(&function_id) {
            if function.is_sync {
                let task_id = self.next_task_id.fetch_add(1, Ordering::Relaxed);
                function.function.call(task_id, parameter_data);
            } else {
                let call = QueuedCall {
                    function_id,
                    parameter_data: Box::from(parameter_data),
                    completion_info: None,
                };

                self.call_queue.lock().unwrap().push(call);
            }
        }
    }

    pub fn call_async(
        &self,
        function_id: ComponentId,
        async_completion_id: ComponentId,
        parameter_data: &[MaybeUninit<u8>],
        user_data: Box<[MaybeUninit<u8>]>,
    ) {
        if let Some(function) = self.platform_functions.get(&function_id) {
            if function.is_sync {
                let task_id = self.next_task_id.fetch_add(1, Ordering::Relaxed);
                function.function.call(task_id, parameter_data);

                let task_info = InFlightTask {
                    async_completion_id,
                    user_data,
                };

                self.in_flight_tasks
                    .lock()
                    .unwrap()
                    .insert(task_id, task_info);
            } else {
                let completion_info = QueuedCompletionInfo {
                    async_completion_id,
                    user_data,
                };

                let call = QueuedCall {
                    function_id,
                    parameter_data: Box::from(parameter_data),
                    completion_info: Some(completion_info),
                };

                self.call_queue.lock().unwrap().push(call);
            }
        }
    }

    /// Returns all received completions for a given `AsyncCompletion`.
    pub fn completions(&self, async_completion_id: ComponentId) -> &[CompletedTask] {
        self.completions
            .get(&async_completion_id)
            .map_or(&[], |vec| vec.as_slice())
    }
}
