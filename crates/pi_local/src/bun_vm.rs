//! Narrow ABI for the source-linked Bun library. The worker owns one VM/thread
//! for process lifetime; no CLI entry, executable subprocess or dlclose.
use anyhow::{Result, anyhow, ensure};
use std::ffi::{CStr, c_void};

type Init = unsafe extern "C" fn() -> i32;
type Eval = unsafe extern "C" fn(*const u8, usize, *mut u8, usize, *mut usize) -> i32;
type Pump = unsafe extern "C" fn(u32, u32) -> i32;
const OUTPUT_LIMIT: usize = 24 * 1024 * 1024;
pub(crate) const INPUT_LIMIT: usize = 16 * 1024 * 1024;

pub(crate) struct Vm {
    // Deliberately retained. Unloading or reinitializing Bun/JSC is unsupported.
    _library: *mut c_void,
    eval: Eval,
    pump: Pump,
    output: Vec<u8>,
}
impl Vm {
    pub(crate) fn open() -> Result<Self> {
        // SAFETY: the name is a static C string. Android resolves it inside the
        // app's native-library namespace; only a bundled shared object is used.
        let library = unsafe {
            libc::dlopen(
                c"libbunembeddingpoc.so".as_ptr(),
                libc::RTLD_NOW | libc::RTLD_LOCAL,
            )
        };
        ensure!(
            !library.is_null(),
            "Embedded Bun library could not be loaded"
        );
        let symbol = |name: &CStr| -> Result<*mut c_void> {
            // SAFETY: the retained handle is valid and name is NUL-terminated.
            let pointer = unsafe { libc::dlsym(library, name.as_ptr()) };
            ensure!(!pointer.is_null(), "Embedded Bun bridge ABI is incomplete");
            Ok(pointer)
        };
        // SAFETY: these exact signatures are defined in the separately source-
        // linked experiments/bun-embedding/native/embedding.rs bridge.
        let init: Init = unsafe { std::mem::transmute(symbol(c"pi_bun_init")?) };
        let eval: Eval = unsafe { std::mem::transmute(symbol(c"pi_bun_eval")?) };
        let pump: Pump = unsafe { std::mem::transmute(symbol(c"pi_bun_pump")?) };
        // SAFETY: initialization and all subsequent calls stay on this worker.
        ensure!(unsafe { init() } == 0, "Embedded Bun initialization failed");
        Ok(Self {
            _library: library,
            eval,
            pump,
            output: vec![0; OUTPUT_LIMIT],
        })
    }
    pub(crate) fn evaluate(&mut self, source: &str) -> Result<String> {
        ensure!(
            source.len() <= INPUT_LIMIT,
            "Embedded request exceeds the runtime limit"
        );
        let mut written = 0;
        // SAFETY: valid UTF-8 input and disjoint caller-owned output storage;
        // calls are serialized on the VM's owning thread.
        let status = unsafe {
            (self.eval)(
                source.as_ptr(),
                source.len(),
                self.output.as_mut_ptr(),
                self.output.len(),
                &mut written,
            )
        };
        // Evaluation may already have side effects. NEVER automatically retry
        // an insufficient-output or failed call (in particular an outbox drain).
        ensure!(
            status == 0 && written <= self.output.len(),
            "Embedded JavaScript operation failed ({status})"
        );
        String::from_utf8(self.output[..written].to_vec())
            .map_err(|_| anyhow!("Invalid embedded runtime response"))
    }
    pub(crate) fn tick(&mut self) -> Result<()> {
        // SAFETY: owning worker; budgets are cooperative between trusted turns.
        let status = unsafe { (self.pump)(4, 2000) };
        ensure!(status >= 0, "Embedded runtime pump failed");
        Ok(())
    }
}
