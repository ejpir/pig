//! Experimental trusted-code bridge mounted into the pinned Bun source tree.
//! One permanent thread/VM; cooperative async pump, no CLI dispatch/unload/sandbox.
use core::{cell::Cell, ffi::c_char, ptr};
use std::sync::atomic::{AtomicBool, Ordering};
use bun_jsc::{JSGlobalObject, JSValue};
use bun_jsc::virtual_machine::{InitOptions, VirtualMachine};

static CLAIMED: AtomicBool = AtomicBool::new(false);
#[thread_local]
static EMBED_VM: Cell<*mut VirtualMachine> = Cell::new(ptr::null_mut());
unsafe extern "C" {
    safe fn Bun__useMimallocForICU();
    fn Bun__REPL__evaluate(global: *const JSGlobalObject, source: *const u8, source_len: usize,
        filename: *const u8, filename_len: usize, exception: *mut JSValue) -> JSValue;
}

#[unsafe(no_mangle)]
pub(crate) extern "C" fn pi_bun_init() -> i32 {
    if !EMBED_VM.get().is_null() { return 0; }
    if CLAIMED.swap(true, Ordering::AcqRel) { return -1; }
    let argv = Box::leak(Box::new([c"bun-jni".as_ptr(),ptr::null::<c_char>()]));
    // SAFETY: both the strings and argv array have process lifetime. This is
    // the first Bun/its private ICU initialization, on its permanent thread.
    unsafe { bun_core::init_argv(1,argv.as_ptr()); Bun__useMimallocForICU(); }
    bun_core::output::stdio::init();
    bun_core::StackCheck::configure_thread();
    bun_core::set_start_time(bun_core::time::nano_timestamp());
    bun_jsc::initialize(bun_jsc::InitializeOptions {
        eval_mode:true,one_shot:false,short_lived_globals:false,
    });
    bun_ast::initialize_store();
    let vm = match VirtualMachine::init(InitOptions {
        eval_mode:true,is_main_thread:true,..Default::default()
    }) {Ok(vm)=>vm,Err(_)=>return -2};
    // SAFETY: init returns a fresh, thread-owned VM. The static path lives for
    // the entire process; this experiment deliberately never destroys the VM.
    unsafe {(*vm).set_main(b"/__bun_jni_probe__.js");}
    bun_jsc::virtual_machine::IS_MAIN_THREAD_VM.set(true);
    EMBED_VM.set(vm);
    0
}

/// Evaluate a trusted UTF-8 synchronous Program; copy the completion value as
/// UTF-8 into a caller-owned buffer. No returned pointers/allocator crossings.
///
/// # Safety
/// Inputs must be valid nonoverlapping buffers on the initialized VM thread.
#[unsafe(no_mangle)]
pub(crate) unsafe extern "C" fn pi_bun_eval(source:*const u8, source_len:usize, out:*mut u8,
    capacity:usize, written:*mut usize) -> i32 {
    if source.is_null() || out.is_null() || written.is_null() || source_len>16*1024*1024 {return -1;}
    // SAFETY: caller supplies valid output length storage.
    unsafe {*written=0;}
    let vm=EMBED_VM.get();if vm.is_null(){return -2;}
    // SAFETY: the thread-owned VM and its Bun global remain alive; do not hold
    // an exclusive VM borrow across JavaScript execution.
    let global=unsafe{&*(*vm).global};
    let _api_lock=global.vm().get_api_lock();
    let mut exception=JSValue::ZERO;
    // SAFETY: source is valid UTF-8 and lives through the synchronous call;
    // the exception slot and filename are valid for the call.
    let value=unsafe{Bun__REPL__evaluate(global,source,source_len,b"[jni]".as_ptr(),5,&raw mut exception)};
    if !exception.is_empty() || value.is_empty(){return -3;}
    let _root=value.protected();
    let utf8=match value.to_utf8(global){Ok(bytes)=>bytes,Err(_)=>{
        let _=global.clear_exception_except_termination();EMBED_VM.set(ptr::null_mut());return -4;
    }};
    let bytes=utf8.slice();
    // SAFETY: length storage is valid; no output bytes copied if capacity is insufficient.
    unsafe{*written=bytes.len();}
    if bytes.len()>capacity{return 1;}
    // SAFETY: nonoverlapping output buffer has at least bytes.len() capacity.
    unsafe{ptr::copy_nonoverlapping(bytes.as_ptr(),out,bytes.len());}
    0
}

/// SAFETY: owning permanent worker, API lock held, no reentrant bridge call.
unsafe fn pump_one(vm: *mut VirtualMachine) {
    // SAFETY: VM, loop and timers belong to this worker and live for the process.
    // The poll is zero-wait. Budgets apply between turns, not inside trusted JS.
    unsafe {
        let el = (*vm).event_loop;
        (*el).tick();
        (*el).tick_immediate_tasks(vm);
        (*el).promote_yield_tasks();
        let io = (*el).usockets_loop();
        #[cfg(unix)]
        {
            let count = (*vm).pending_unref_counter.swap(0, Ordering::Relaxed);
            if count > 0 { (*io).unref_count(count); }
        }
        let timers = crate::jsc_hooks::timer_all();
        if !timers.is_null() { (*timers).update_date_header_timer_if_necessary(&*io, vm); }
        (*el).run_imminent_gc_timer();
        (*io).tick_without_idle();
        #[cfg(unix)]
        if !timers.is_null() { (*timers).drain_timers(vm.cast()); }
        (*vm).on_after_event_loop();
        (*el).tick();
    }
}

/// Cooperative, zero-kernel-wait pump. Does not bound individual callbacks or
/// synchronous native work; this development bridge evaluates only trusted code.
#[unsafe(no_mangle)]
pub(crate) extern "C" fn pi_bun_pump(max_turns: u32, budget_us: u32) -> i32 {
    let vm = EMBED_VM.get();
    if vm.is_null() { return -2; }
    // SAFETY: access is serialized on the thread owning this process-lifetime VM.
    let global = unsafe { &*(*vm).global };
    let _api_lock = global.vm().get_api_lock();
    let start = std::time::Instant::now();
    let budget = std::time::Duration::from_micros(u64::from(budget_us));
    let mut turns = 0;
    while turns < max_turns.min(64) && start.elapsed() < budget {
        // SAFETY: same VM worker and API lock throughout the turn.
        if unsafe { !(*vm).script_allowed() } { return -3; }
        unsafe { pump_one(vm); }
        turns += 1;
    }
    turns as i32
}
