//! Portable declarative mini-app definitions and a bounded, capability-free QuickJS runtime.

pub mod graphics;
pub mod input;
pub mod profile;
mod program;
pub mod prompt;
pub mod reply;
mod svg;
mod svg_path;
pub mod trace;
pub use graphics::{Animation, Scene, Shape};
pub use input::{InputEvent, InputState};

use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::{Map, Value};
use std::{
    cell::RefCell,
    collections::HashMap,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, serde::Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    pub title: String,
    pub initial_state: serde_json::Value,
    pub view: Node,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub animation: Option<Animation>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layer {
    pub draw: String,
    /// None depends on all state; Some([]) is static. Only declared fields are supplied.
    pub deps: Option<Vec<String>>,
}

#[derive(Clone, Debug, serde::Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Node {
    Column {
        gap: f32,
        width: Option<f32>,
        children: Vec<Node>,
    },
    Row {
        gap: f32,
        children: Vec<Node>,
    },
    Text {
        text: Option<String>,
        bind: Option<String>,
        style: String,
    },
    Button {
        label: String,
        action: String,
        tone: String,
        #[serde(default)]
        hold: Option<String>,
    },
    Slider {
        label: String,
        bind: String,
        min: f64,
        max: f64,
        step: f64,
        action: String,
    },
    Canvas {
        label: String,
        height: f32,
        view_box: [f32; 4],
        draw: String,
        #[serde(default)]
        pointer_action: Option<String>,
        #[serde(default)]
        input_action: Option<String>,
        #[serde(default)]
        layers: Vec<Layer>,
    },
}

impl Node {
    fn validate(&self, depth: usize, count: &mut usize) -> Result<()> {
        *count += 1;
        ensure!(depth <= 12 && *count <= 128, "View tree is too large");
        match self {
            Self::Column {
                gap,
                width,
                children,
            } => {
                ensure!(gap.is_finite() && (0.0..=64.0).contains(gap), "Invalid gap");
                if let Some(width) = width {
                    ensure!(
                        width.is_finite() && (40.0..=600.0).contains(width),
                        "Invalid width"
                    );
                }
                for child in children {
                    child.validate(depth + 1, count)?;
                }
            }
            Self::Row { gap, children } => {
                ensure!(gap.is_finite() && (0.0..=64.0).contains(gap), "Invalid gap");
                for child in children {
                    child.validate(depth + 1, count)?;
                }
            }
            Self::Text { text, bind, style } => {
                ensure!(
                    text.is_some() != bind.is_some(),
                    "Text needs exactly one of text/bind"
                );
                ensure!(
                    text.as_ref().is_none_or(|s| s.len() <= 256),
                    "Text too long"
                );
                ensure!(
                    bind.as_ref().is_none_or(|s| s.len() <= 64),
                    "Binding too long"
                );
                ensure!(
                    ["display", "expression", "heading", "history", "muted"]
                        .contains(&style.as_str()),
                    "Unknown text style"
                );
            }
            Self::Slider {
                label,
                bind,
                min,
                max,
                step,
                action,
            } => {
                ensure!(
                    label.len() <= 64 && !bind.is_empty() && bind.len() <= 64 && action.len() <= 32,
                    "Invalid slider label/binding/action"
                );
                ensure!(
                    min.is_finite()
                        && max.is_finite()
                        && step.is_finite()
                        && min < max
                        && *step > 0.0
                        && *step <= max - min
                        && (max - min).is_finite()
                        && ((max - min) / step).is_finite(),
                    "Invalid slider range"
                );
            }
            Self::Canvas {
                label,
                height,
                view_box,
                draw,
                pointer_action,
                input_action,
                layers,
            } => {
                ensure!(
                    pointer_action.is_none() || input_action.is_none(),
                    "Use onPointer or onInput, not both"
                );
                ensure!(
                    input_action
                        .as_ref()
                        .is_none_or(|a| !a.is_empty() && a.len() <= 32),
                    "Invalid input callback"
                );
                ensure!(layers.len() <= 8, "Too many canvas layers");
                for layer in layers {
                    ensure!(
                        !layer.draw.is_empty() && layer.draw.len() <= 32,
                        "Invalid layer callback"
                    );
                    ensure!(
                        layer.deps.as_ref().is_none_or(|d| d.len() <= 64),
                        "Too many layer dependencies"
                    );
                }
                ensure!(
                    !label.is_empty() && label.len() <= 128,
                    "Canvas needs a short accessible label"
                );
                ensure!(
                    height.is_finite() && (120.0..=480.0).contains(height),
                    "Invalid canvas height"
                );
                ensure!(
                    view_box
                        .iter()
                        .all(|n| n.is_finite() && n.abs() <= 1_000_000.0)
                        && view_box[2] >= 1.0
                        && view_box[3] >= 1.0,
                    "Invalid view_box"
                );
                ensure!(
                    !draw.is_empty() && draw.len() <= 32,
                    "Invalid draw callback id"
                );
                ensure!(
                    pointer_action
                        .as_ref()
                        .is_none_or(|a| !a.is_empty() && a.len() <= 32),
                    "Invalid pointer action"
                );
            }
            Self::Button {
                label,
                action,
                tone,
                hold,
            } => {
                ensure!(
                    hold.as_ref().is_none_or(|h| !h.is_empty() && h.len() <= 32),
                    "Invalid held button name"
                );
                ensure!(
                    hold.is_none() || !label.contains(['\n', '\r']),
                    "Held button label must be one line"
                );
                ensure!(label.len() <= 32 && action.len() <= 32, "Button too long");
                ensure!(
                    ["accent", "operator", "utility", "normal"].contains(&tone.as_str()),
                    "Unknown button tone"
                );
            }
        }
        Ok(())
    }
}

impl Node {
    fn children(&self) -> &[Node] {
        match self {
            Self::Column { children, .. } | Self::Row { children, .. } => children,
            _ => &[],
        }
    }

    fn has_pointer(&self, action: &str) -> bool {
        matches!(self, Self::Canvas { pointer_action: Some(a), .. } if a == action)
            || self.children().iter().any(|n| n.has_pointer(action))
    }

    fn has_input(&self, action: &str) -> bool {
        matches!(self, Self::Canvas { input_action: Some(a), .. } if a == action)
            || matches!(self, Self::Button { action: a, hold: Some(_), .. } if a == action)
            || self.children().iter().any(|n| n.has_input(action))
    }

    fn hold_name(&self, id: &str) -> Option<&str> {
        if let Self::Button {
            action,
            hold: Some(name),
            ..
        } = self
            && action == id
        {
            return Some(name);
        }
        self.children().iter().find_map(|n| n.hold_name(id))
    }

    fn layers(&self, name: &str) -> Option<&[Layer]> {
        if let Self::Canvas { draw, layers, .. } = self
            && draw == name
        {
            return Some(layers);
        }
        self.children().iter().find_map(|n| n.layers(name))
    }

    fn has_draw(&self, name: &str) -> bool {
        matches!(self, Self::Canvas { draw, .. } if draw == name)
            || self.children().iter().any(|n| n.has_draw(name))
    }

    fn validate_bindings(
        &self,
        state: &Map<String, Value>,
        canvases: &mut usize,
        sliders: &mut std::collections::HashSet<String>,
    ) -> Result<()> {
        match self {
            Self::Canvas { layers, .. } => {
                for layer in layers {
                    if let Some(deps) = &layer.deps {
                        let mut seen = std::collections::HashSet::new();
                        ensure!(
                            deps.iter().all(|d| state.contains_key(d) && seen.insert(d)),
                            "Unknown or duplicate layer dependency"
                        );
                    }
                }
                *canvases += 1;
                ensure!(*canvases <= 4, "Too many canvases");
            }
            Self::Slider { bind, min, max, .. } => {
                let value = state
                    .get(bind.as_str())
                    .and_then(Value::as_f64)
                    .context("Slider must bind a float")?;
                ensure!(
                    (*min..=*max).contains(&value),
                    "Initial slider value is out of range"
                );
                ensure!(
                    sliders.insert(bind.clone()),
                    "Slider bindings must be unique"
                );
            }
            _ => {}
        }
        for child in self.children() {
            child.validate_bindings(state, canvases, sliders)?;
        }
        Ok(())
    }

    fn slider(&self, name: &str) -> Option<(f64, f64, f64, String)> {
        if let Self::Slider {
            bind,
            min,
            max,
            step,
            action,
            ..
        } = self
            && bind == name
        {
            return Some((*min, *max, *step, action.clone()));
        }
        self.children().iter().find_map(|n| n.slider(name))
    }
}

pub const MAX_BUNDLE: usize = 65_536;
const CALL_TIME: Duration = Duration::from_millis(20);
const INIT_TIME: Duration = Duration::from_millis(50);

/// Rust-owned state/definition only. No guest closures, globals or jobs survive calls.
pub struct Runtime {
    pub definition: Definition,
    program: program::Program,
    registration: String,
    state: Map<String, Value>,
    profile: profile::Profile,
    scenes: RefCell<HashMap<String, Result<Arc<Scene>, String>>>,
    last_scenes: RefCell<HashMap<String, Arc<Scene>>>,
    texts: RefCell<HashMap<String, Result<String, String>>>,
    text_error: RefCell<Option<String>>,
    layers: RefCell<HashMap<Layer, (Value, Arc<Scene>)>>,
    input: InputState,
}

enum Code<'a> {
    Source(&'a str),
    Cached(&'a program::Program),
}

fn execute(
    code: Code<'_>,
    budget: Duration,
    operation: Option<(&str, &str, &Value, &Value)>,
) -> Result<(String, String, Option<program::Program>)> {
    use rquickjs::context::intrinsic;
    use rquickjs::{Context as JsContext, Function, Runtime as JsRuntime};
    // Declared before the VM: includes its ordinary teardown on every path.
    let _execute_trace = trace::scope(trace::Phase::Execute);
    let deadline = Instant::now() + budget;
    let setup_trace = trace::scope(trace::Phase::CreateVm);
    let rt = JsRuntime::new().map_err(|e| anyhow::anyhow!("JavaScript runtime: {e}"))?;
    rt.set_memory_limit(16 * 1024 * 1024);
    rt.set_max_stack_size(256 * 1024);
    let promised = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let tracker = promised.clone();
    rt.set_promise_hook(Some(Box::new(move |_, kind, _, _| {
        if matches!(kind, rquickjs::promise::PromiseHookType::Init) {
            tracker.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    })));
    rt.set_interrupt_handler(Some(Box::new(move || Instant::now() >= deadline)));
    let context = JsContext::custom::<(intrinsic::Eval, intrinsic::Json, intrinsic::Promise)>(&rt)
        .map_err(|e| anyhow::anyhow!("JavaScript context: {e}"))?;
    drop(setup_trace);
    let result = context
        .with(
            |ctx| -> rquickjs::Result<(String, String, Option<program::Program>)> {
                let result = (|| {
                    let compiled = match code {
                        Code::Source(source) => Some(program::Program::compile(&ctx, source)?),
                        Code::Cached(_) => None,
                    };
                    let program = match &compiled {
                        Some(program) => program,
                        None => match code {
                            Code::Cached(program) => program,
                            _ => unreachable!(),
                        },
                    };
                    let bridge = program.register(&ctx, deadline)?;
                    let definition: String = {
                        let _trace = trace::scope(trace::Phase::Definition);
                        let definition: Function = bridge.get("definition")?;
                        definition.call(())?
                    };
                    let output = if let Some((mode, id, state, args)) = operation {
                        // Whole trusted bridge: Rust serialization, guest operation,
                        // bridge validation/output serialization; not pure guest time.
                        let _trace = trace::scope(trace::Phase::Bridge);
                        let invoke: Function = bridge.get("invoke")?;
                        invoke.call((mode, id, state.to_string(), args.to_string()))?
                    } else {
                        String::new()
                    };
                    Ok((definition, output, compiled))
                })();
                // Read error messages only while the same time/memory guard is armed.
                result.map_err(|error| {
                    let _trace = trace::scope(trace::Phase::Exception);
                    if matches!(error, rquickjs::Error::Exception) {
                        let thrown = ctx.catch();
                        let message = thrown
                            .as_exception()
                            .and_then(|e| e.message())
                            .unwrap_or_else(|| "JavaScript exception".into());
                        rquickjs::Error::new_from_js_message(
                            "exception",
                            "visual",
                            message.chars().take(512).collect::<String>(),
                        )
                    } else {
                        error
                    }
                })
            },
        )
        .map_err(|e| anyhow::anyhow!("JavaScript failed: {e}"));
    let _checks_trace = trace::scope(trace::Phase::Checks);
    ensure!(Instant::now() < deadline, "JavaScript time limit exceeded");
    ensure!(
        !promised.load(std::sync::atomic::Ordering::Relaxed) && !rt.is_job_pending(),
        "Async jobs are not supported; callbacks must be synchronous"
    );
    let result = result?;
    ensure!(
        result.0.len() <= 32_768 && result.1.len() <= 128 * 1024,
        "JavaScript output too large"
    );
    Ok(result)
}

impl Runtime {
    pub fn from_bundle(source: &str) -> Result<Self> {
        ensure!(source.len() <= MAX_BUNDLE, "Visual app exceeds 64 KiB");
        ensure!(
            !source.trim_start().starts_with('{'),
            "Legacy JSON/Rhai visual: regenerate with @Visualize for JavaScript. Source and saved data are retained."
        );
        let (registration, _, program) = execute(Code::Source(source), INIT_TIME, None)?;
        let validation_trace = trace::scope(trace::Phase::ValidateDefinition);
        let definition: Definition =
            serde_json::from_str(&registration).context("Invalid native UI definition")?;
        ensure!(
            !definition.title.is_empty() && definition.title.len() <= 80,
            "Invalid title"
        );
        definition.view.validate(0, &mut 0)?;
        let state = definition
            .initial_state
            .as_object()
            .context("State must be a flat object")?
            .clone();
        validate_flat(&state)?;
        definition
            .view
            .validate_bindings(&state, &mut 0, &mut std::collections::HashSet::new())?;
        if let Some(animation) = &definition.animation {
            animation.validate()?;
            ensure!(
                state.get(&animation.running).is_some_and(Value::is_boolean),
                "Animation running must bind a boolean"
            );
        }
        drop(validation_trace);
        Ok(Self {
            definition,
            program: program.expect("Source execution compiles the program"),
            registration,
            state,
            profile: profile::Profile::default(),
            scenes: RefCell::new(HashMap::new()),
            last_scenes: RefCell::new(HashMap::new()),
            texts: RefCell::new(HashMap::new()),
            text_error: RefCell::new(None),
            layers: RefCell::new(HashMap::new()),
            input: InputState::default(),
        })
    }
    pub fn load(folder: &Path) -> Result<Self> {
        use std::io::Read;
        let path = folder.join("app.js");
        ensure!(
            std::fs::symlink_metadata(&path)?.file_type().is_file(),
            "App source must be a regular file, not a symlink"
        );
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
        }
        let file = options.open(path)?;
        ensure!(
            file.metadata()?.is_file(),
            "App source must be a regular file"
        );
        let mut bytes = Vec::new();
        file.take((MAX_BUNDLE + 1) as u64).read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= MAX_BUNDLE, "Visual app exceeds 64 KiB");
        Self::from_bundle(&String::from_utf8(bytes).context("Source must be UTF-8")?)
    }
    pub fn state_json(&self) -> Result<Value> {
        Ok(Value::Object(self.state.clone()))
    }
    fn validate_state(&self, next: &Map<String, Value>) -> Result<()> {
        validate_flat(next)?;
        ensure!(next.len() == self.state.len(), "State schema changed");
        for (key, value) in next {
            let previous = self.state.get(key).context("Unknown state field")?;
            ensure!(
                (previous.is_number() && value.is_number())
                    || (previous.is_boolean() && value.is_boolean())
                    || (previous.is_string() && value.is_string()),
                "State field type changed: {key}"
            );
        }
        Ok(())
    }
    fn invalidate(&self) {
        self.scenes.borrow_mut().clear();
        self.texts.borrow_mut().clear();
        self.text_error.borrow_mut().take();
    }
    fn commit(&mut self, next: Map<String, Value>) -> Result<()> {
        self.validate_state(&next)?;
        let same = next.iter().all(|(key, value)| {
            let previous = &self.state[key];
            if value.is_number() {
                previous.as_f64() == value.as_f64()
            } else {
                previous == value
            }
        });
        if !same {
            self.state = next;
            self.invalidate();
        }
        Ok(())
    }
    pub fn restore_state(&mut self, state: &Value) -> Result<()> {
        ensure!(
            serde_json::to_vec(state)?.len() <= 32_768,
            "Saved state too large"
        );
        self.commit(
            state
                .as_object()
                .context("Saved state must be an object")?
                .clone(),
        )
    }
    fn invoke(&self, mode: &str, id: &str, state: &Value, args: Value) -> Result<String> {
        self.invoke_bounded(mode, id, state, args, CALL_TIME)
    }
    fn invoke_bounded(
        &self,
        mode: &str,
        id: &str,
        state: &Value,
        args: Value,
        budget: Duration,
    ) -> Result<String> {
        let (definition, result, _) = execute(
            Code::Cached(&self.program),
            budget,
            Some((mode, id, state, &args)),
        )?;
        ensure!(
            definition == self.registration,
            "App registration must be deterministic"
        );
        Ok(result)
    }
    fn event(&mut self, id: &str, state: Value, args: Value) -> Result<()> {
        let _timing = self.profile.timer(profile::Stage::Callback);
        let output = self.invoke("event", id, &state, args)?;
        let next: Value = serde_json::from_str(&output)?;
        self.commit(
            next.as_object()
                .context("Callback state must be an object")?
                .clone(),
        )
    }
    pub fn dispatch(&mut self, id: &str) -> Result<()> {
        self.event(id, self.state_json()?, serde_json::json!([]))
    }
    pub fn number(&self, key: &str) -> Option<f64> {
        self.state.get(key).and_then(Value::as_f64)
    }
    pub fn slider_value(&self, bind: &str, value: f64) -> Result<f64> {
        let (min, max, step, _) = self
            .definition
            .view
            .slider(bind)
            .context("Unknown slider")?;
        ensure!(value.is_finite(), "Invalid slider value");
        let value = (min + ((value.clamp(min, max) - min) / step).round() * step).clamp(min, max);
        ensure!(value.is_finite(), "Invalid slider value");
        Ok(value)
    }
    pub fn slide(&mut self, bind: &str, value: f64) -> Result<()> {
        let (_, _, _, action) = self
            .definition
            .view
            .slider(bind)
            .context("Unknown slider")?;
        let value = self.slider_value(bind, value)?;
        let mut state = self.state_json()?;
        state[bind] = serde_json::json!(value);
        self.event(&action, state, serde_json::json!([value]))
    }
    pub fn animating(&self) -> bool {
        self.definition.animation.as_ref().is_some_and(|a| {
            self.state
                .get(&a.running)
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
    }
    pub fn tick(&mut self, dt: f64) -> Result<()> {
        ensure!(
            dt.is_finite() && (0.0..=0.1).contains(&dt),
            "Invalid tick duration"
        );
        if self.animating() {
            self.event(
                "__tick",
                self.state_json()?,
                serde_json::json!([dt, self.input_snapshot()]),
            )
        } else {
            Ok(())
        }
    }
    /// Input state reflects the physical transition even if a guest handler fails.
    pub fn input(&mut self, id: &str, event: InputEvent) -> Result<()> {
        ensure!(self.definition.view.has_input(id), "Unknown input action");
        if matches!(&event, InputEvent::Key { down: false, .. })
            && self.input.key_target.as_deref() != Some(id)
        {
            return Ok(());
        }
        self.input.apply(id, &event)?;
        if self.definition.view.hold_name(id).is_some() {
            return Ok(());
        }
        self.event(
            id,
            self.state_json()?,
            serde_json::json!([event, self.input_snapshot()]),
        )
    }
    pub fn button_held(&self, id: &str) -> bool {
        self.input.pointers.iter().any(|p| p.target == id)
            || (self.input.key_target.as_deref() == Some(id)
                && self.input.keys.iter().any(|k| k == "space" || k == "enter"))
    }
    fn input_snapshot(&self) -> Value {
        let mut snapshot = serde_json::to_value(&self.input).expect("finite validated input");
        let mut buttons = std::collections::BTreeSet::new();
        for target in self
            .input
            .pointers
            .iter()
            .map(|p| p.target.as_str())
            .chain(self.input.key_target.as_deref())
        {
            if self.button_held(target)
                && let Some(name) = self.definition.view.hold_name(target)
            {
                buttons.insert(name);
            }
        }
        snapshot["buttons"] = serde_json::json!(buttons);
        snapshot
    }
    pub fn clear_input(&mut self) {
        self.input = InputState::default();
    }
    pub fn cancel_input(&mut self) {
        let mut targets: std::collections::BTreeSet<_> = self
            .input
            .pointers
            .iter()
            .map(|p| p.target.clone())
            .collect();
        targets.extend(self.input.key_target.clone());
        self.clear_input();
        for target in targets {
            let _ = self.input(&target, InputEvent::Cancel);
        }
    }
    pub fn input_state(&self) -> &InputState {
        &self.input
    }

    pub fn pointer(&mut self, id: &str, x: f64, y: f64) -> Result<()> {
        ensure!(
            self.definition.view.has_pointer(id),
            "Unknown pointer action"
        );
        ensure!(
            x.is_finite() && y.is_finite() && x.abs() <= 2_000_000. && y.abs() <= 2_000_000.,
            "Invalid pointer event"
        );
        self.event(id, self.state_json()?, serde_json::json!([x, y]))
    }
    pub fn text(&self, id: &str) -> String {
        if let Some(error) = self.text_error.borrow().as_ref() {
            let _trace = trace::scope(trace::Phase::TextFailure);
            return format!("Text failed: {error}");
        }
        if let Some(value) = self.state.get(id) {
            let _trace = trace::scope(trace::Phase::TextDirect);
            return value
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| value.to_string());
        }
        if let Some(cached) = self.texts.borrow().get(id) {
            let _trace = trace::scope(trace::Phase::TextHit);
            return cached
                .clone()
                .unwrap_or_else(|e| format!("Text failed: {e}"));
        }
        let _text_miss_trace = trace::scope(trace::Phase::TextMiss);
        let result = (|| -> Result<Map<String, Value>> {
            let _timing = self.profile.timer(profile::Stage::TextScript);
            let value = self.invoke("texts", "", &self.state_json()?, serde_json::json!([]))?;
            let _trace = trace::scope(trace::Phase::DecodeTexts);
            Ok(serde_json::from_str(&value)?)
        })();
        match result {
            Ok(values) => {
                let mut texts = self.texts.borrow_mut();
                for (key, value) in values {
                    texts.insert(key, Ok(value.as_str().unwrap_or_default().to_owned()));
                }
                texts
                    .get(id)
                    .cloned()
                    .unwrap_or_else(|| Err("Unknown text binding".into()))
                    .unwrap_or_else(|e| format!("Text failed: {e}"))
            }
            Err(error) => {
                let error = format!("{error:#}");
                *self.text_error.borrow_mut() = Some(error.clone());
                format!("Text failed: {error}")
            }
        }
    }
    pub fn scene(&self, id: &str) -> Result<Arc<Scene>> {
        ensure!(self.definition.view.has_draw(id), "Unknown draw function");
        if let Some(cached) = self.scenes.borrow().get(id) {
            let _trace = trace::scope(trace::Phase::SceneHit);
            self.profile.cache(true);
            return cached.clone().map_err(anyhow::Error::msg);
        }
        let _scene_miss_trace = trace::scope(trace::Phase::SceneMiss);
        self.profile.cache(false);
        let result = (|| -> Result<Arc<Scene>> {
            let layers = self.definition.view.layers(id).unwrap_or_default();
            if !layers.is_empty() {
                return self.layered_scene(layers);
            }
            let timing = self.profile.timer(profile::Stage::DrawScript);
            let output = self.invoke("draw", id, &self.state_json()?, serde_json::json!([]))?;
            drop(timing);
            let _decode = self.profile.timer(profile::Stage::Decode);
            let scene: Scene = {
                let _trace = trace::scope(trace::Phase::DecodeScene);
                serde_json::from_str(&output).context("Invalid scene data")?
            };
            {
                let _trace = trace::scope(trace::Phase::ValidateScene);
                graphics::validate_scene(&scene)?;
            }
            let scene = Arc::new(scene);
            self.last_scenes
                .borrow_mut()
                .insert(id.into(), scene.clone());
            Ok(scene)
        })()
        .map_err(|e| format!("{e:#}"));
        if let Ok(scene) = &result {
            self.last_scenes
                .borrow_mut()
                .insert(id.into(), scene.clone());
        }
        self.scenes.borrow_mut().insert(id.into(), result.clone());
        result.map_err(anyhow::Error::msg)
    }
    fn layered_scene(&self, layers: &[Layer]) -> Result<Arc<Scene>> {
        let mut missing = Vec::new();
        let mut keys = Vec::new();
        for layer in layers {
            let key = match &layer.deps {
                None => self.state_json()?,
                Some(deps) => Value::Object(
                    deps.iter()
                        .map(|k| (k.clone(), self.state[k].clone()))
                        .collect(),
                ),
            };
            if !self
                .layers
                .borrow()
                .get(layer)
                .is_some_and(|(cached, _)| *cached == key)
            {
                missing.push(layer);
            }
            keys.push(key);
        }
        let mut fresh = HashMap::new();
        let deadline = Instant::now() + CALL_TIME;
        for layer in missing {
            let _timing = self.profile.timer(profile::Stage::DrawScript);
            let snapshot = match &layer.deps {
                None => self.state_json()?,
                Some(deps) => Value::Object(
                    deps.iter()
                        .map(|k| (k.clone(), self.state[k].clone()))
                        .collect(),
                ),
            };
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .context("Layer drawing time limit exceeded")?;
            // Fresh context per layer: skipping a cached layer must not change another
            // callback's globals/closures. All misses share one total invocation budget.
            let output = self.invoke_bounded(
                "draw",
                &layer.draw,
                &snapshot,
                serde_json::json!([]),
                remaining,
            )?;
            let part: Scene = serde_json::from_str(&output)?;
            graphics::validate_scene(&part)?;
            fresh.insert(layer.clone(), part);
        }
        let mut scene = Vec::new();
        let mut replacements = Vec::new();
        let cache = self.layers.borrow();
        for (layer, key) in layers.iter().zip(keys) {
            let part = if let Some(part) = fresh.get(layer) {
                graphics::validate_scene(part)?;
                let part = Arc::new(part.clone());
                replacements.push((layer.clone(), (key, part.clone())));
                part
            } else {
                cache.get(layer).context("Missing drawing layer")?.1.clone()
            };
            scene.extend(part.iter().cloned());
        }
        graphics::validate_scene(&scene)?;
        drop(cache);
        self.layers.borrow_mut().extend(replacements);
        Ok(Arc::new(scene))
    }

    pub fn drawing_failed(&self) -> bool {
        self.text_error.borrow().is_some() || self.scenes.borrow().values().any(Result::is_err)
    }
    pub fn last_scene(&self, id: &str) -> Option<Arc<Scene>> {
        self.last_scenes.borrow().get(id).cloned()
    }
    pub fn enable_profiling(&self, enabled: bool) {
        self.profile.enable(enabled);
    }
    pub fn take_profile(&self) -> profile::Batch {
        self.profile.take()
    }
}
fn validate_flat(state: &Map<String, Value>) -> Result<()> {
    ensure!(state.len() <= 64, "State exceeds 64 fields");
    for (key, value) in state {
        ensure!(key.len() <= 64, "State field name too long");
        ensure!(
            value.is_boolean() || value.is_number() || value.is_string(),
            "State must contain finite primitives"
        );
        if let Some(text) = value.as_str() {
            ensure!(text.len() <= 256, "State string too long");
        }
        if value.is_number() {
            ensure!(
                value.as_f64().is_some_and(f64::is_finite),
                "Invalid state number"
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const COUNTER: &str = r#"app({title:'Counter',state:{count:0},view:ui.column([
        ui.text(s=>String(s.count),{style:'display'}),
        ui.button('Add',s=>{s.count++;},{id:'add'})
    ])});"#;
    #[test]
    fn registration_mismatch_preserves_state_and_computed_cache() {
        let mut app = Runtime::from_bundle(COUNTER).unwrap();
        let Node::Column { children, .. } = &app.definition.view else {
            panic!()
        };
        let Node::Text { bind: Some(id), .. } = &children[0] else {
            panic!()
        };
        let id = id.clone();
        assert_eq!(app.text(&id), "0");
        let before = app.state_json().unwrap();
        app.registration = "unexpected registration".into();
        assert!(app.dispatch("add").is_err());
        assert_eq!(app.state_json().unwrap(), before);
        assert_eq!(app.text(&id), "0");
        assert_eq!(app.texts.borrow().len(), 1);
    }

    #[test]
    fn javascript_counter_uses_native_nodes_and_transactional_state() {
        let mut app = Runtime::from_bundle(COUNTER).unwrap();
        let Node::Column { children, .. } = &app.definition.view else {
            panic!()
        };
        let Node::Text {
            bind: Some(bind), ..
        } = &children[0]
        else {
            panic!()
        };
        let bind = bind.clone();
        assert_eq!(app.text(&bind), "0");
        app.dispatch("add").unwrap();
        assert_eq!(app.text(&bind), "1");
        assert_eq!(app.state_json().unwrap(), serde_json::json!({"count":1}));
    }
}
