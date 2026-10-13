//! Process-local QuickJS bytecode, never loaded from disk or provided by guests.
//!
//! Parsing is cached, not execution: each invocation still executes registration
//! in a new guarded runtime/context, preserving rollback of closures and globals.
use rquickjs::{Ctx, Error, FromJs, Object, Value, qjs};
use std::{
    ffi::CString,
    sync::{Arc, OnceLock},
};

const MAX_BYTECODE: usize = 512 * 1024;
static BOOTSTRAP: OnceLock<Arc<Bytecode>> = OnceLock::new();

// ReadObject's owned reference must already have unwind protection when a
// trusted diagnostic hook runs/drops. No hooks run between relinquishing this
// reference and EvalFunction consuming it.
struct ReadValue<'a, 'js> {
    ctx: &'a Ctx<'js>,
    value: Option<qjs::JSValue>,
}
impl ReadValue<'_, '_> {
    fn into_raw(mut self) -> qjs::JSValue {
        self.value.take().expect("owned bytecode value")
    }
}
impl Drop for ReadValue<'_, '_> {
    fn drop(&mut self) {
        if let Some(value) = self.value.take() {
            // SAFETY: one owned reference from ReadObject, same live context.
            unsafe { qjs::JS_FreeValue(self.ctx.as_raw().as_ptr(), value) };
        }
    }
}

struct Bytecode(Vec<u8>);
impl Bytecode {
    fn compile(ctx: &Ctx<'_>, source: &str) -> rquickjs::Result<Self> {
        let _trace = crate::trace::scope(crate::trace::Phase::Compile);
        let source = CString::new(source)?;
        // SAFETY: context and null-terminated source are live. COMPILE_ONLY
        // produces an owned value, wrapped immediately for refcounted cleanup.
        let value = unsafe {
            Value::from_raw(
                ctx.clone(),
                qjs::JS_Eval(
                    ctx.as_raw().as_ptr(),
                    source.as_ptr(),
                    source.as_bytes().len() as _,
                    c"pi-visual.js".as_ptr(),
                    (qjs::JS_EVAL_TYPE_GLOBAL
                        | qjs::JS_EVAL_FLAG_STRICT
                        | qjs::JS_EVAL_FLAG_COMPILE_ONLY) as _,
                ),
            )
        };
        if value.is_exception() {
            return Err(Error::Exception);
        }
        let mut len = 0;
        // SAFETY: WriteObject borrows the live compiled value and returns a
        // QuickJS-owned buffer, valid for len bytes until js_free in this context.
        let bytes = unsafe {
            let buffer = qjs::JS_WriteObject(
                ctx.as_raw().as_ptr(),
                &mut len,
                value.as_raw(),
                qjs::JS_WRITE_OBJ_BYTECODE as _,
            );
            if buffer.is_null() {
                return Err(Error::Exception);
            }
            if len > MAX_BYTECODE as _ {
                qjs::js_free(ctx.as_raw().as_ptr(), buffer.cast());
                return Err(Error::new_from_js_message(
                    "bytecode",
                    "visual",
                    "Compiled app exceeds 512 KiB",
                ));
            }
            let bytes = std::slice::from_raw_parts(buffer, len as usize).to_vec();
            qjs::js_free(ctx.as_raw().as_ptr(), buffer.cast());
            bytes
        };
        Ok(Self(bytes))
    }
    fn eval<'js>(&self, ctx: &Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        // SAFETY: bytes come exclusively from WriteObject in this process with
        // this pinned engine/architecture. There is no public/file bytecode
        // input or mutable bytecode access. ReadObject returns an owned value.
        let read_trace = crate::trace::scope(crate::trace::Phase::BytecodeRead);
        let compiled = ReadValue {
            ctx,
            value: Some(unsafe {
                qjs::JS_ReadObject(
                    ctx.as_raw().as_ptr(),
                    self.0.as_ptr(),
                    self.0.len() as _,
                    qjs::JS_READ_OBJ_BYTECODE as _,
                )
            }),
        };
        // A named owner is initialized before dropping the diagnostic guard.
        drop(read_trace);
        if unsafe { qjs::JS_IsException(compiled.value.expect("owned bytecode value")) } {
            return Err(Error::Exception);
        }
        // SAFETY: EvalFunction consumes the owned ReadObject result; its owned
        // result belongs to this same live context and is wrapped immediately.
        let eval_trace = crate::trace::scope(crate::trace::Phase::BytecodeEval);
        let value = unsafe {
            Value::from_raw(
                ctx.clone(),
                qjs::JS_EvalFunction(ctx.as_raw().as_ptr(), compiled.into_raw()),
            )
        };
        drop(eval_trace);
        if value.is_exception() {
            return Err(Error::Exception);
        }
        Ok(value)
    }
}

pub(super) struct Program {
    bootstrap: Arc<Bytecode>,
    source: Bytecode,
}
impl Program {
    pub(super) fn compile(ctx: &Ctx<'_>, source: &str) -> rquickjs::Result<Self> {
        let bootstrap = match BOOTSTRAP.get() {
            Some(bootstrap) => bootstrap.clone(),
            None => {
                let bootstrap = Arc::new(Bytecode::compile(ctx, include_str!("api.js"))?);
                // Concurrent initialization may win; either immutable result is
                // equivalent and valid. No contexts/values escape compilation.
                let _ = BOOTSTRAP.set(bootstrap.clone());
                bootstrap
            }
        };
        Ok(Self {
            bootstrap,
            source: Bytecode::compile(ctx, source)?,
        })
    }
    pub(super) fn register<'js>(&self, ctx: &Ctx<'js>) -> rquickjs::Result<Object<'js>> {
        let bootstrap_trace = crate::trace::scope(crate::trace::Phase::Bootstrap);
        let bridge = Object::from_js(ctx, self.bootstrap.eval(ctx)?)?;
        drop(bootstrap_trace);
        {
            let _trace = crate::trace::scope(crate::trace::Phase::AppRegistration);
            self.source.eval(ctx)?;
        }
        Ok(bridge)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn producer_size_rejection_does_not_poison_the_next_compilation() {
        let rt = rquickjs::Runtime::new().unwrap();
        rt.set_memory_limit(16 * 1024 * 1024);
        let context = rquickjs::Context::custom::<rquickjs::context::intrinsic::Eval>(&rt).unwrap();
        // This private producer test exceeds the public 64KiB source cap on
        // purpose; no arbitrary/malformed bytes are ever handed to the reader.
        let source = format!("'{}'", "x".repeat(MAX_BYTECODE + 1));
        context.with(|ctx| {
            for _ in 0..3 {
                assert!(Bytecode::compile(&ctx, &source).is_err());
                let good = Bytecode::compile(&ctx, "40+2").unwrap();
                assert_eq!(good.eval(&ctx).unwrap().as_int(), Some(42));
            }
        });
    }
}
