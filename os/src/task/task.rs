//! Types related to task management

use super::TaskContext;

/// Per-task syscall counters used by `sys_trace`.
#[derive(Copy, Clone)]
pub struct SyscallCounter {
    /// Number of `write` syscalls issued by the task.
    pub write: usize,
    /// Number of `exit` syscalls issued by the task.
    pub exit: usize,
    /// Number of `yield` syscalls issued by the task.
    pub yield_: usize,
    /// Number of `get_time` syscalls issued by the task.
    pub get_time: usize,
    /// Number of `trace` syscalls issued by the task.
    pub trace: usize,
}

/// The task control block (TCB) of a task.
#[derive(Copy, Clone)]
pub struct TaskControlBlock {
    /// The task status in it's lifecycle
    pub task_status: TaskStatus,
    /// The task context
    pub task_cx: TaskContext,
    /// syscall statistics of this task
    pub syscall_counter: SyscallCounter,
}

/// The status of a task
#[derive(Copy, Clone, PartialEq)]
pub enum TaskStatus {
    /// uninitialized
    UnInit,
    /// ready to run
    Ready,
    /// running
    Running,
    /// exited
    Exited,
}
