//! Trusted, process-local diagnostics only. Never exposed to visual scripts.
//! Typed phases carry no source, state, identifiers or timing-clock policy.
use std::{any::Any, sync::OnceLock};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Execute,
    CreateVm,
    Compile,
    Bootstrap,
    AppRegistration,
    BytecodeRead,
    BytecodeEval,
    Definition,
    Bridge,
    Exception,
    Checks,
    ValidateDefinition,
    SceneHit,
    SceneMiss,
    DecodeScene,
    ValidateScene,
    TextDirect,
    TextHit,
    TextMiss,
    TextFailure,
    DecodeTexts,
}

/// A trusted host factory. Return None when tracing is disabled. The opaque
/// guard drops synchronously on the calling thread; it must not retain VM data.
/// Installation is once per process, independent of per-runtime HUD profiling.
/// Factories/destructors are trusted host code and must not panic or reenter a VM.
pub type ScopeFactory = fn(Phase) -> Option<Box<dyn Any>>;
static FACTORY: OnceLock<ScopeFactory> = OnceLock::new();

pub fn install(factory: ScopeFactory) -> bool {
    FACTORY.set(factory).is_ok()
}

// Box<dyn Any> is intentionally not Send/Sync. No guard crosses an await or is
// stored in a VM/cache. Android allocates the small guard only with shell tracing.
pub(crate) struct Scope {
    _guard: Option<Box<dyn Any>>,
}
pub(crate) fn scope(phase: Phase) -> Scope {
    scoped(phase, FACTORY.get().copied())
}
fn scoped(phase: Phase, factory: Option<ScopeFactory>) -> Scope {
    Scope {
        _guard: factory.and_then(|f| f(phase)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    thread_local! {
        static EVENTS: RefCell<Vec<(Phase, bool)>> = const { RefCell::new(Vec::new()) };
    }
    thread_local! {
        static RECORD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
        static PANIC_AT: std::cell::Cell<Option<(Phase, bool)>> = const { std::cell::Cell::new(None) };
    }
    fn maybe_panic(phase: Phase, begin: bool) {
        if PANIC_AT.with(|p| p.get() == Some((phase, begin))) {
            PANIC_AT.with(|p| p.set(None));
            panic!("synthetic trusted diagnostic unwind");
        }
    }
    struct End(Phase);
    impl Drop for End {
        fn drop(&mut self) {
            EVENTS.with(|e| e.borrow_mut().push((self.0, false)));
            maybe_panic(self.0, false);
        }
    }
    fn begin(phase: Phase) -> Option<Box<dyn Any>> {
        EVENTS.with(|e| e.borrow_mut().push((phase, true)));
        Some(Box::new(End(phase)))
    }
    fn disabled(_: Phase) -> Option<Box<dyn Any>> {
        None
    }

    #[test]
    fn absent_and_disabled_factories_retain_no_guard() {
        assert!(scoped(Phase::Execute, None)._guard.is_none());
        assert!(scoped(Phase::Execute, Some(disabled))._guard.is_none());
    }
    #[test]
    fn scopes_balance_nested_early_return_and_unwind_on_the_same_thread() {
        EVENTS.with(|e| e.borrow_mut().clear());
        fn operation() -> Result<(), ()> {
            let _outer = scoped(Phase::Execute, Some(begin));
            let _inner = scoped(Phase::Bridge, Some(begin));
            Err(())
        }
        assert!(operation().is_err());
        assert_eq!(
            EVENTS.with(|e| e.borrow().clone()),
            vec![
                (Phase::Execute, true),
                (Phase::Bridge, true),
                (Phase::Bridge, false),
                (Phase::Execute, false),
            ]
        );
        EVENTS.with(|e| e.borrow_mut().clear());
        let _ = std::panic::catch_unwind(|| {
            let _scope = scoped(Phase::Exception, Some(begin));
            panic!("host-only diagnostic unwind");
        });
        assert_eq!(
            EVENTS.with(|e| e.borrow().clone()),
            vec![(Phase::Exception, true), (Phase::Exception, false)]
        );
    }
    fn recording(phase: Phase) -> Option<Box<dyn Any>> {
        if !RECORD.with(|r| r.get()) {
            return None;
        }
        maybe_panic(phase, true);
        begin(phase)
    }
    #[test]
    fn actual_vm_cache_and_error_paths_keep_phase_guards_balanced() {
        assert!(install(recording));
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                RECORD.with(|r| r.set(false));
                PANIC_AT.with(|p| p.set(None));
            }
        }
        let _reset = Reset;
        RECORD.with(|r| r.set(true));
        EVENTS.with(|e| e.borrow_mut().clear());
        let runtime = crate::Runtime::from_bundle(
            r#"app({title:'Phases',state:{x:1},view:ui.column([
            ui.text(s=>String(s.x)),
            ui.canvas({id:'plot',label:'Plot',height:120,viewBox:[0,0,10,10],draw(g,s){}})
        ])});"#,
        )
        .unwrap();
        let crate::Node::Column { children, .. } = &runtime.definition.view else {
            panic!()
        };
        let crate::Node::Text {
            bind: Some(bind), ..
        } = &children[0]
        else {
            panic!()
        };
        let crate::Node::Canvas { draw, .. } = &children[1] else {
            panic!()
        };
        assert_eq!(runtime.text(bind), "1");
        assert_eq!(runtime.text(bind), "1");
        assert_eq!(runtime.text("x"), "1");
        runtime.scene(draw).unwrap();
        runtime.scene(draw).unwrap();
        assert!(crate::Runtime::from_bundle("invalid JavaScript !!").is_err());
        let events = EVENTS.with(|e| e.borrow().clone());
        for phase in [
            Phase::Execute,
            Phase::CreateVm,
            Phase::Bootstrap,
            Phase::AppRegistration,
            Phase::BytecodeRead,
            Phase::BytecodeEval,
            Phase::Definition,
            Phase::Bridge,
            Phase::ValidateDefinition,
            Phase::SceneMiss,
            Phase::SceneHit,
            Phase::DecodeScene,
            Phase::ValidateScene,
            Phase::TextMiss,
            Phase::TextHit,
            Phase::TextDirect,
            Phase::DecodeTexts,
            Phase::Exception,
        ] {
            assert!(events.contains(&(phase, true)), "missing {phase:?}");
        }
        let mut stack = Vec::new();
        for (phase, begin) in events {
            if begin {
                stack.push(phase);
            } else {
                assert_eq!(stack.pop(), Some(phase));
            }
        }
        assert!(stack.is_empty());
        // The raw owned ReadObject value is protected before either hook can
        // unwind. Recovery must not trip QuickJS's nonempty-GC teardown check.
        for point in [
            (Phase::BytecodeRead, false),
            (Phase::BytecodeEval, true),
            (Phase::BytecodeEval, false),
            (Phase::Bootstrap, false),
        ] {
            PANIC_AT.with(|p| p.set(Some(point)));
            assert!(
                std::panic::catch_unwind(|| {
                    let _ = crate::Runtime::from_bundle(
                        "app({title:'Unwind',state:{},view:ui.text('ok')});",
                    );
                })
                .is_err()
            );
            assert!(PANIC_AT.with(|p| p.get().is_none()));
            assert!(
                crate::Runtime::from_bundle("app({title:'Recovery',state:{},view:ui.text('ok')});")
                    .is_ok()
            );
        }
    }
}
