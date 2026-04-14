use crate::sync::{Condvar, Mutex, MutexBlocking, MutexSpin, Semaphore};
use crate::task::{block_current_and_run_next, current_process, current_task};
use crate::timer::{add_timer, get_time_ms};
use alloc::collections::BTreeSet;
use alloc::sync::Arc;

const EDEADLK: isize = -0xdead;

fn current_tid() -> usize {
    current_task()
        .unwrap()
        .inner_exclusive_access()
        .res
        .as_ref()
        .unwrap()
        .tid
}

fn has_mutex_cycle(inner: &crate::task::ProcessControlBlockInner, start_tid: usize) -> bool {
    fn dfs(
        inner: &crate::task::ProcessControlBlockInner,
        start_tid: usize,
        cur_tid: usize,
        visited: &mut BTreeSet<usize>,
    ) -> bool {
        if !visited.insert(cur_tid) {
            return false;
        }
        if let Some(&mid) = inner.thread_wait_mutex.get(&cur_tid) {
            if let Some(Some(owner_tid)) = inner.mutex_owner.get(mid) {
                if *owner_tid == start_tid {
                    return true;
                }
                if dfs(inner, start_tid, *owner_tid, visited) {
                    return true;
                }
            }
        }
        false
    }
    let mut visited = BTreeSet::new();
    dfs(inner, start_tid, start_tid, &mut visited)
}

fn semaphore_state_is_safe(inner: &crate::task::ProcessControlBlockInner) -> bool {
    let sem_n = inner.semaphore_available.len();
    let mut tids = BTreeSet::new();
    for alloc in inner.semaphore_allocation.iter() {
        for (&tid, &cnt) in alloc.iter() {
            if cnt > 0 {
                tids.insert(tid);
            }
        }
    }
    for &tid in inner.thread_wait_semaphore.keys() {
        tids.insert(tid);
    }
    if tids.is_empty() {
        return true;
    }

    let tids: alloc::vec::Vec<usize> = tids.into_iter().collect();
    let mut work = inner.semaphore_available.clone();
    let mut finish = alloc::vec![false; tids.len()];
    let mut changed = true;
    while changed {
        changed = false;
        for (i, tid) in tids.iter().enumerate() {
            if finish[i] {
                continue;
            }
            let mut can_finish = true;
            if let Some(wait_sid) = inner.thread_wait_semaphore.get(tid) {
                if *wait_sid >= sem_n || work[*wait_sid] < 1 {
                    can_finish = false;
                }
            }
            if !can_finish {
                continue;
            }
            for sid in 0..sem_n {
                if let Some(cnt) = inner.semaphore_allocation[sid].get(tid) {
                    work[sid] += *cnt as isize;
                }
            }
            finish[i] = true;
            changed = true;
        }
    }
    finish.into_iter().all(|f| f)
}
/// sleep syscall
pub fn sys_sleep(ms: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_sleep",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let expire_ms = get_time_ms() + ms;
    let task = current_task().unwrap();
    add_timer(expire_ms, task);
    block_current_and_run_next();
    0
}
/// mutex create syscall
pub fn sys_mutex_create(blocking: bool) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_mutex_create",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mutex: Option<Arc<dyn Mutex>> = if !blocking {
        Some(Arc::new(MutexSpin::new()))
    } else {
        Some(Arc::new(MutexBlocking::new()))
    };
    let mut process_inner = process.inner_exclusive_access();
    if let Some(id) = process_inner
        .mutex_list
        .iter()
        .enumerate()
        .find(|(_, item)| item.is_none())
        .map(|(id, _)| id)
    {
        process_inner.mutex_list[id] = mutex;
        if id >= process_inner.mutex_owner.len() {
            process_inner.mutex_owner.resize(id + 1, None);
        }
        process_inner.mutex_owner[id] = None;
        id as isize
    } else {
        process_inner.mutex_list.push(mutex);
        process_inner.mutex_owner.push(None);
        process_inner.mutex_list.len() as isize - 1
    }
}
/// mutex lock syscall
pub fn sys_mutex_lock(mutex_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_mutex_lock",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let tid = current_tid();
    let mut process_inner = process.inner_exclusive_access();
    if mutex_id >= process_inner.mutex_list.len() || process_inner.mutex_list[mutex_id].is_none() {
        return -1;
    }
    if mutex_id >= process_inner.mutex_owner.len() {
        process_inner.mutex_owner.resize(mutex_id + 1, None);
    }
    if process_inner.deadlock_detect_enabled && process_inner.mutex_owner[mutex_id].is_some() {
        process_inner.thread_wait_mutex.insert(tid, mutex_id);
        if has_mutex_cycle(&process_inner, tid) {
            process_inner.thread_wait_mutex.remove(&tid);
            return EDEADLK;
        }
    }
    let mutex = Arc::clone(process_inner.mutex_list[mutex_id].as_ref().unwrap());
    drop(process_inner);
    drop(process);
    mutex.lock();
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    if mutex_id < process_inner.mutex_owner.len() {
        process_inner.mutex_owner[mutex_id] = Some(tid);
    }
    process_inner.thread_wait_mutex.remove(&tid);
    0
}
/// mutex unlock syscall
pub fn sys_mutex_unlock(mutex_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_mutex_unlock",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let tid = current_tid();
    let process_inner = process.inner_exclusive_access();
    if mutex_id >= process_inner.mutex_list.len() || process_inner.mutex_list[mutex_id].is_none() {
        return -1;
    }
    let mutex = Arc::clone(process_inner.mutex_list[mutex_id].as_ref().unwrap());
    drop(process_inner);
    drop(process);
    mutex.unlock();
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    if mutex_id < process_inner.mutex_owner.len() && process_inner.mutex_owner[mutex_id] == Some(tid)
    {
        process_inner.mutex_owner[mutex_id] = None;
    }
    0
}
/// semaphore create syscall
pub fn sys_semaphore_create(res_count: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_semaphore_create",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let id = if let Some(id) = process_inner
        .semaphore_list
        .iter()
        .enumerate()
        .find(|(_, item)| item.is_none())
        .map(|(id, _)| id)
    {
        process_inner.semaphore_list[id] = Some(Arc::new(Semaphore::new(res_count)));
        if id >= process_inner.semaphore_available.len() {
            process_inner.semaphore_available.resize(id + 1, 0);
        }
        process_inner.semaphore_available[id] = res_count as isize;
        if id >= process_inner.semaphore_allocation.len() {
            process_inner
                .semaphore_allocation
                .resize_with(id + 1, Default::default);
        }
        process_inner.semaphore_allocation[id].clear();
        id
    } else {
        process_inner
            .semaphore_list
            .push(Some(Arc::new(Semaphore::new(res_count))));
        process_inner.semaphore_available.push(res_count as isize);
        process_inner.semaphore_allocation.push(Default::default());
        process_inner.semaphore_list.len() - 1
    };
    id as isize
}
/// semaphore up syscall
pub fn sys_semaphore_up(sem_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_semaphore_up",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let tid = current_tid();
    let process_inner = process.inner_exclusive_access();
    if sem_id >= process_inner.semaphore_list.len() || process_inner.semaphore_list[sem_id].is_none() {
        return -1;
    }
    let sem = Arc::clone(process_inner.semaphore_list[sem_id].as_ref().unwrap());
    drop(process_inner);
    sem.up();
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    if sem_id >= process_inner.semaphore_available.len() {
        process_inner.semaphore_available.resize(sem_id + 1, 0);
    }
    let sem_count = {
        let sem_inner = sem.inner.exclusive_access();
        sem_inner.count
    };
    process_inner.semaphore_available[sem_id] = sem_count.max(0);
    if sem_id >= process_inner.semaphore_allocation.len() {
        process_inner
            .semaphore_allocation
            .resize_with(sem_id + 1, Default::default);
    }
    if let Some(cnt) = process_inner.semaphore_allocation[sem_id].get_mut(&tid) {
        if *cnt > 0 {
            *cnt -= 1;
        }
        if *cnt == 0 {
            process_inner.semaphore_allocation[sem_id].remove(&tid);
        }
    }
    0
}
/// semaphore down syscall
pub fn sys_semaphore_down(sem_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_semaphore_down",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let tid = current_tid();
    let mut process_inner = process.inner_exclusive_access();
    if sem_id >= process_inner.semaphore_list.len() || process_inner.semaphore_list[sem_id].is_none() {
        return -1;
    }
    if sem_id >= process_inner.semaphore_available.len() {
        process_inner.semaphore_available.resize(sem_id + 1, 0);
    }
    if sem_id >= process_inner.semaphore_allocation.len() {
        process_inner
            .semaphore_allocation
            .resize_with(sem_id + 1, Default::default);
    }
    let sem = Arc::clone(process_inner.semaphore_list[sem_id].as_ref().unwrap());
    let sem_count = {
        let sem_inner = sem.inner.exclusive_access();
        sem_inner.count
    };
    process_inner.semaphore_available[sem_id] = sem_count.max(0);
    if process_inner.deadlock_detect_enabled && sem_count <= 0 {
        process_inner.thread_wait_semaphore.insert(tid, sem_id);
        if !semaphore_state_is_safe(&process_inner) {
            process_inner.thread_wait_semaphore.remove(&tid);
            return EDEADLK;
        }
    }
    drop(process_inner);
    sem.down();
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    process_inner.thread_wait_semaphore.remove(&tid);
    if sem_id >= process_inner.semaphore_available.len() {
        process_inner.semaphore_available.resize(sem_id + 1, 0);
    }
    let sem_count = {
        let sem_inner = sem.inner.exclusive_access();
        sem_inner.count
    };
    process_inner.semaphore_available[sem_id] = sem_count.max(0);
    if sem_id >= process_inner.semaphore_allocation.len() {
        process_inner
            .semaphore_allocation
            .resize_with(sem_id + 1, Default::default);
    }
    let entry = process_inner.semaphore_allocation[sem_id].entry(tid).or_insert(0);
    *entry += 1;
    0
}
/// condvar create syscall
pub fn sys_condvar_create() -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_condvar_create",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let id = if let Some(id) = process_inner
        .condvar_list
        .iter()
        .enumerate()
        .find(|(_, item)| item.is_none())
        .map(|(id, _)| id)
    {
        process_inner.condvar_list[id] = Some(Arc::new(Condvar::new()));
        id
    } else {
        process_inner
            .condvar_list
            .push(Some(Arc::new(Condvar::new())));
        process_inner.condvar_list.len() - 1
    };
    id as isize
}
/// condvar signal syscall
pub fn sys_condvar_signal(condvar_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_condvar_signal",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let condvar = Arc::clone(process_inner.condvar_list[condvar_id].as_ref().unwrap());
    drop(process_inner);
    condvar.signal();
    0
}
/// condvar wait syscall
pub fn sys_condvar_wait(condvar_id: usize, mutex_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_condvar_wait",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let condvar = Arc::clone(process_inner.condvar_list[condvar_id].as_ref().unwrap());
    let mutex = Arc::clone(process_inner.mutex_list[mutex_id].as_ref().unwrap());
    drop(process_inner);
    condvar.wait(mutex);
    0
}
/// enable deadlock detection syscall
///
/// YOUR JOB: Implement deadlock detection, but might not all in this syscall
pub fn sys_enable_deadlock_detect(_enabled: usize) -> isize {
    trace!("kernel: sys_enable_deadlock_detect {}", _enabled);
    if _enabled > 1 {
        return -1;
    }
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    process_inner.deadlock_detect_enabled = _enabled == 1;
    if !process_inner.deadlock_detect_enabled {
        process_inner.thread_wait_mutex.clear();
        process_inner.thread_wait_semaphore.clear();
    }
    0
}
