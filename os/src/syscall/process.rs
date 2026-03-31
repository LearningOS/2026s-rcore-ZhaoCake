//! Process management syscalls
use crate::mm::{translated_byte_buffer, MapPermission, PageTable, PTEFlags, StepByOne, VirtAddr};
use crate::task::{
    change_program_brk, current_mmap, current_munmap, current_syscall_count, current_user_token,
    exit_current_and_run_next, suspend_current_and_run_next,
};
use crate::timer::get_time_us;
use core::mem::size_of;

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

fn user_addr_has_perm(addr: usize, perm: PTEFlags) -> bool {
    let page_table = PageTable::from_token(current_user_token());
    let pte = if let Some(pte) = page_table.translate(VirtAddr::from(addr).floor()) {
        pte
    } else {
        return false;
    };
    pte.is_valid() && pte.flags().contains(PTEFlags::U) && pte.flags().contains(perm)
}

fn user_range_has_perm(start: usize, len: usize, perm: PTEFlags) -> bool {
    let end = if let Some(end) = start.checked_add(len) {
        end
    } else {
        return false;
    };
    if len == 0 {
        return true;
    }
    let start_vpn = VirtAddr::from(start).floor();
    let end_vpn = VirtAddr::from(end).ceil();
    let page_table = PageTable::from_token(current_user_token());
    let mut vpn = start_vpn;
    while vpn != end_vpn {
        let pte = if let Some(pte) = page_table.translate(vpn) {
            pte
        } else {
            return false;
        };
        if !pte.is_valid() || !pte.flags().contains(PTEFlags::U) || !pte.flags().contains(perm) {
            return false;
        }
        vpn.step();
    }
    true
}

/// task exits and submit an exit code
pub fn sys_exit(_exit_code: i32) -> isize {
    trace!("kernel: sys_exit");
    exit_current_and_run_next();
    -1
}

/// current task gives up resources for other tasks
pub fn sys_yield() -> isize {
    trace!("kernel: sys_yield");
    suspend_current_and_run_next();
    0
}

/// YOUR JOB: get time with second and microsecond
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TimeVal`] is splitted by two pages ?
pub fn sys_get_time(ts: *mut TimeVal, _tz: usize) -> isize {
    trace!("kernel: sys_get_time");
    let ts_addr = ts as usize;
    let len = size_of::<TimeVal>();
    if !user_range_has_perm(ts_addr, len, PTEFlags::W) {
        return -1;
    }
    let us = get_time_us();
    let tv = TimeVal {
        sec: us / 1_000_000,
        usec: us % 1_000_000,
    };
    let src = unsafe { core::slice::from_raw_parts((&tv as *const TimeVal) as *const u8, len) };
    let mut offset = 0;
    for dst in translated_byte_buffer(current_user_token(), ts as *const u8, len) {
        let copy_len = dst.len();
        dst.copy_from_slice(&src[offset..offset + copy_len]);
        offset += copy_len;
    }
    0
}

/// TODO: Finish sys_trace to pass testcases
/// HINT: You might reimplement it with virtual memory management.
pub fn sys_trace(trace_request: usize, id: usize, data: usize) -> isize {
    trace!("kernel: sys_trace");
    match trace_request {
        0 => {
            if !user_addr_has_perm(id, PTEFlags::R) {
                return -1;
            }
            let pte = PageTable::from_token(current_user_token())
                .translate(VirtAddr::from(id).floor())
                .unwrap();
            let pa = (pte.ppn().0 << 12) + (id & 0xfff);
            unsafe { (pa as *const u8).read_volatile() as isize }
        }
        1 => {
            if !user_addr_has_perm(id, PTEFlags::W) {
                return -1;
            }
            let pte = PageTable::from_token(current_user_token())
                .translate(VirtAddr::from(id).floor())
                .unwrap();
            let pa = (pte.ppn().0 << 12) + (id & 0xfff);
            unsafe {
                (pa as *mut u8).write_volatile(data as u8);
            }
            0
        }
        2 => current_syscall_count(id),
        _ => -1,
    }
}

// YOUR JOB: Implement mmap.
pub fn sys_mmap(start: usize, len: usize, prot: usize) -> isize {
    trace!("kernel: sys_mmap");
    if !VirtAddr::from(start).aligned() {
        return -1;
    }
    if prot & !0x7 != 0 || prot & 0x7 == 0 {
        return -1;
    }
    let mut perm = MapPermission::U;
    if prot & 0x1 != 0 {
        perm |= MapPermission::R;
    }
    if prot & 0x2 != 0 {
        perm |= MapPermission::W;
    }
    if prot & 0x4 != 0 {
        perm |= MapPermission::X;
    }
    if current_mmap(VirtAddr::from(start), len, perm) {
        0
    } else {
        -1
    }
}

// YOUR JOB: Implement munmap.
pub fn sys_munmap(start: usize, len: usize) -> isize {
    trace!("kernel: sys_munmap");
    if !VirtAddr::from(start).aligned() {
        return -1;
    }
    if current_munmap(VirtAddr::from(start), len) {
        0
    } else {
        -1
    }
}
/// change data segment size
pub fn sys_sbrk(size: i32) -> isize {
    trace!("kernel: sys_sbrk");
    if let Some(old_brk) = change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}
