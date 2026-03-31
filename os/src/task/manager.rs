//!Implementation of [`TaskManager`]
use super::TaskControlBlock;
use crate::sync::UPSafeCell;
use alloc::collections::VecDeque;
use alloc::sync::Arc;
use lazy_static::*;

const BIG_STRIDE: usize = 0x7fff_ffff;

fn stride_less(a: usize, b: usize) -> bool {
    let diff = a.wrapping_sub(b);
    diff > (usize::MAX >> 1)
}
///A array of `TaskControlBlock` that is thread-safe
pub struct TaskManager {
    ready_queue: VecDeque<Arc<TaskControlBlock>>,
}

/// A simple stride scheduler.
impl TaskManager {
    ///Creat an empty TaskManager
    pub fn new() -> Self {
        Self {
            ready_queue: VecDeque::new(),
        }
    }
    /// Add process back to ready queue
    pub fn add(&mut self, task: Arc<TaskControlBlock>) {
        self.ready_queue.push_back(task);
    }
    /// Take a process out of the ready queue
    pub fn fetch(&mut self) -> Option<Arc<TaskControlBlock>> {
        if self.ready_queue.is_empty() {
            return None;
        }
        let mut min_idx = 0usize;
        let mut min_stride = usize::MAX;
        for (idx, task) in self.ready_queue.iter().enumerate() {
            let task_inner = task.inner_exclusive_access();
            let stride = task_inner.stride;
            drop(task_inner);
            if idx == 0 || stride_less(stride, min_stride) {
                min_stride = stride;
                min_idx = idx;
            }
        }
        let task = self.ready_queue.remove(min_idx).unwrap();
        let mut task_inner = task.inner_exclusive_access();
        let pass = (BIG_STRIDE / task_inner.priority).max(1);
        task_inner.stride = task_inner.stride.wrapping_add(pass);
        drop(task_inner);
        Some(task)
    }
}

lazy_static! {
    /// TASK_MANAGER instance through lazy_static!
    pub static ref TASK_MANAGER: UPSafeCell<TaskManager> =
        unsafe { UPSafeCell::new(TaskManager::new()) };
}

/// Add process to ready queue
pub fn add_task(task: Arc<TaskControlBlock>) {
    //trace!("kernel: TaskManager::add_task");
    TASK_MANAGER.exclusive_access().add(task);
}

/// Take a process out of the ready queue
pub fn fetch_task() -> Option<Arc<TaskControlBlock>> {
    //trace!("kernel: TaskManager::fetch_task");
    TASK_MANAGER.exclusive_access().fetch()
}
