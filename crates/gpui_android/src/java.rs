//! The Java half of the activity, `dev.pi.gpui.GpuiActivity` (see `java/`):
//! the on-screen keyboard, window insets, the clipboard, file pickers, opening
//! links, the display's refresh rate, notifications and the URLs the app is
//! opened with, which Android offers only to Java.
//!
//! Java calls in on its UI thread; those calls are queued and wake the main
//! loop. Calls out run on the main thread. Under a plain `NativeActivity` the
//! first call fails, is logged, and these features stay off.

use crate::{
    activity::Notification,
    ime::{ImeState, Push},
};
use android_activity::{AndroidApp, AndroidAppWaker};
use jni::{
    Env, EnvUnowned, JValue, JavaVM, jni_sig, jni_str,
    objects::{JByteArray, JClass, JObject, JObjectArray, JString},
    refs::Global,
    sys::{jint, jobject, jstring},
};
use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
    sync::{Mutex, PoisonError},
};

#[derive(Debug)]
pub(crate) enum JavaEvent {
    /// The keyboard's mirror after a batch of edits, or after applying push `push_id`.
    Ime {
        seq: i32,
        push_id: i32,
        state: ImeState,
    },
    /// The keyboard's action key (done, send, go…): `EditorInfo.IME_ACTION_*`.
    EditorAction(i32),
    /// System bars with display cutouts, and the keyboard, in device pixels:
    /// left, top, right, bottom.
    Insets { bars: [i32; 4], ime: [i32; 4] },
    /// The paths a file pick produced; `None` when the user cancelled.
    Picked {
        request: i32,
        result: Result<Option<Vec<String>>, String>,
    },
    /// The app was opened with a URL: a link or a notification.
    OpenUrl(String),
}

struct Inbox {
    events: Vec<JavaEvent>,
    waker: Option<AndroidAppWaker>,
}

static INBOX: Mutex<Inbox> = Mutex::new(Inbox {
    events: Vec::new(),
    waker: None,
});

fn deliver(event: JavaEvent) {
    let mut inbox = INBOX.lock().unwrap_or_else(PoisonError::into_inner);
    inbox.events.push(event);
    if let Some(waker) = &inbox.waker {
        waker.wake();
    }
}

/// Events from Java since the last call, oldest first.
pub(crate) fn take_events() -> Vec<JavaEvent> {
    std::mem::take(&mut INBOX.lock().unwrap_or_else(PoisonError::into_inner).events)
}

pub(crate) struct Java {
    app: AndroidApp,
    available: Cell<bool>,
}

thread_local! {
    /// The platform's bridge, for [`crate::activity`]'s free functions.
    static CURRENT: RefCell<Weak<Java>> = const { RefCell::new(Weak::new()) };
}

/// The running platform's bridge, on the main thread.
pub(crate) fn current() -> Option<Rc<Java>> {
    CURRENT.with(|current| current.borrow().upgrade())
}

pub(crate) fn set_current(java: &Rc<Java>) {
    CURRENT.with(|current| *current.borrow_mut() = Rc::downgrade(java));
}

impl Java {
    pub fn attach(app: &AndroidApp) -> Self {
        INBOX.lock().unwrap_or_else(PoisonError::into_inner).waker = Some(app.create_waker());
        let java = Self {
            app: app.clone(),
            available: Cell::new(true),
        };
        java.call("attachNative", |env, activity| {
            env.call_method(activity, jni_str!("attachNative"), jni_sig!("()V"), &[])?;
            Ok(())
        });
        java
    }

    /// Whether the activity is `GpuiActivity`, as far as calls so far showed.
    pub fn is_available(&self) -> bool {
        self.available.get()
    }

    /// Runs `f` with the activity, logging the first failure and then staying quiet.
    fn call<T>(
        &self,
        name: &str,
        f: impl FnOnce(&mut Env, &JObject) -> jni::errors::Result<T>,
    ) -> Option<T> {
        if !self.available.get() {
            return None;
        }
        // SAFETY: android-activity keeps the VM and the activity's global
        // reference alive for as long as `app`.
        let vm = unsafe { JavaVM::from_raw(self.app.vm_as_ptr().cast()) };
        let raw = self.app.activity_as_ptr() as jobject;
        let result = vm.attach_current_thread(|env| -> jni::errors::Result<T> {
            let activity = unsafe { env.as_cast_raw::<Global<JObject>>(&raw)? };
            f(env, activity.as_ref())
        });
        match result {
            Ok(value) => Some(value),
            Err(error) => {
                log::warn!(
                    "{name} failed ({error}); is the activity dev.pi.gpui.GpuiActivity? \
                     The keyboard, clipboard, links and insets are off."
                );
                self.available.set(false);
                None
            }
        }
    }

    pub fn show_keyboard(&self) {
        self.call("showKeyboard", |env, activity| {
            env.call_method(activity, jni_str!("showKeyboard"), jni_sig!("()V"), &[])?;
            Ok(())
        });
    }

    pub fn hide_keyboard(&self) {
        self.call("hideKeyboard", |env, activity| {
            env.call_method(activity, jni_str!("hideKeyboard"), jni_sig!("()V"), &[])?;
            Ok(())
        });
    }

    pub fn configure_keyboard(&self, input_type: i32, ime_options: i32) {
        self.call("configureKeyboard", |env, activity| {
            env.call_method(
                activity,
                jni_str!("configureKeyboard"),
                jni_sig!("(II)V"),
                &[JValue::Int(input_type), JValue::Int(ime_options)],
            )?;
            Ok(())
        });
    }

    /// Sends the app's text to the keyboard's mirror.
    pub fn set_text(&self, push: &Push) {
        let state = &push.state;
        self.call("setText", |env, activity| {
            let text = JString::from_str(env, &state.text)?;
            let (composing_start, composing_end) = state
                .composing
                .as_ref()
                .map_or((-1, -1), |range| (range.start as jint, range.end as jint));
            env.call_method(
                activity,
                jni_str!("setText"),
                jni_sig!("(IILjava/lang/String;IIIIZ)V"),
                &[
                    JValue::Int(push.basis),
                    JValue::Int(push.id),
                    JValue::Object(&text),
                    JValue::Int(state.selection.start as jint),
                    JValue::Int(state.selection.end as jint),
                    JValue::Int(composing_start),
                    JValue::Int(composing_end),
                    JValue::Bool(push.restart),
                ],
            )?;
            Ok(())
        });
    }

    pub fn clipboard_text(&self) -> Option<String> {
        self.call("clipboardText", |env, activity| {
            let text = env
                .call_method(
                    activity,
                    jni_str!("clipboardText"),
                    jni_sig!("()Ljava/lang/String;"),
                    &[],
                )?
                .l()?;
            if text.is_null() {
                return Ok(None);
            }
            let text = env.cast_local::<JString>(text)?;
            Ok(Some(text.try_to_string(env)?))
        })
        .flatten()
    }

    pub fn set_clipboard_text(&self, text: &str) {
        self.call("setClipboardText", |env, activity| {
            let text = JString::from_str(env, text)?;
            env.call_method(
                activity,
                jni_str!("setClipboardText"),
                jni_sig!("(Ljava/lang/String;)V"),
                &[JValue::Object(&text)],
            )?;
            Ok(())
        });
    }

    /// The clipboard's image and its MIME type, if it holds one.
    pub fn clipboard_image(&self) -> Option<(String, Vec<u8>)> {
        self.call("clipboardImage", |env, activity| {
            let mime = env
                .call_method(
                    activity,
                    jni_str!("clipboardImageType"),
                    jni_sig!("()Ljava/lang/String;"),
                    &[],
                )?
                .l()?;
            if mime.is_null() {
                return Ok(None);
            }
            let mime = env.cast_local::<JString>(mime)?.try_to_string(env)?;
            let bytes = env
                .call_method(activity, jni_str!("clipboardImage"), jni_sig!("()[B"), &[])?
                .l()?;
            if bytes.is_null() {
                return Ok(None);
            }
            let bytes = env.cast_local::<JByteArray>(bytes)?;
            Ok(Some((mime, env.convert_byte_array(&bytes)?)))
        })
        .flatten()
    }

    /// Copies an image, with text for apps that paste only text.
    pub fn set_clipboard_image(
        &self,
        bytes: &[u8],
        mime: &str,
        extension: &str,
        text: Option<&str>,
    ) {
        self.call("setClipboardImage", |env, activity| {
            let bytes = env.byte_array_from_slice(bytes)?;
            let mime = JString::from_str(env, mime)?;
            let extension = JString::from_str(env, extension)?;
            let text = match text {
                Some(text) => JString::from_str(env, text)?.into(),
                None => JObject::null(),
            };
            env.call_method(
                activity,
                jni_str!("setClipboardImage"),
                jni_sig!("([BLjava/lang/String;Ljava/lang/String;Ljava/lang/String;)V"),
                &[
                    JValue::Object(&bytes),
                    JValue::Object(&mime),
                    JValue::Object(&extension),
                    JValue::Object(&text),
                ],
            )?;
            Ok(())
        });
    }

    /// Shows the system picker for documents to open; the result arrives as
    /// [`JavaEvent::Picked`]. Returns false without the Java activity.
    pub fn pick_files(&self, request: i32, multiple: bool) -> bool {
        self.call("pickFiles", |env, activity| {
            env.call_method(
                activity,
                jni_str!("pickFiles"),
                jni_sig!("(IZ)V"),
                &[JValue::Int(request), JValue::Bool(multiple)],
            )?;
            Ok(())
        })
        .is_some()
    }

    /// Shows the system picker for a document to save to.
    pub fn pick_save_file(&self, request: i32, suggested_name: &str) -> bool {
        self.call("pickSaveFile", |env, activity| {
            let name = JString::from_str(env, suggested_name)?;
            env.call_method(
                activity,
                jni_str!("pickSaveFile"),
                jni_sig!("(ILjava/lang/String;)V"),
                &[JValue::Int(request), JValue::Object(&name)],
            )?;
            Ok(())
        })
        .is_some()
    }

    pub fn open_url(&self, url: &str) {
        self.call("openUrl", |env, activity| {
            let url = JString::from_str(env, url)?;
            env.call_method(
                activity,
                jni_str!("openUrl"),
                jni_sig!("(Ljava/lang/String;)V"),
                &[JValue::Object(&url)],
            )?;
            Ok(())
        });
    }

    pub fn scan_qr(&self) -> bool {
        self.call("scanQr", |env, activity| {
            env.call_method(activity, jni_str!("scanQr"), jni_sig!("()Z"), &[])?
                .z()
        })
        .unwrap_or(false)
    }

    pub fn show_page(&self, title: &str, html: &str, dark: bool, source: bool, poster: &str) -> bool {
        self.call("showPage", |env, activity| {
            let title = JString::from_str(env, title)?;
            let html = JString::from_str(env, html)?;
            let poster = JString::from_str(env, poster)?;
            env.call_method(
                activity,
                jni_str!("showPage"),
                jni_sig!("(Ljava/lang/String;Ljava/lang/String;ZZLjava/lang/String;)Z"),
                &[
                    JValue::Object(&title),
                    JValue::Object(&html),
                    JValue::Bool(dark),
                    JValue::Bool(source),
                    JValue::Object(&poster),
                ],
            )?
            .z()
        })
        .unwrap_or(false)
    }

    pub fn render_poster(&self, html: &str, path: &str) -> bool {
        self.call("renderPoster", |env, activity| {
            let html = JString::from_str(env, html)?;
            let path = JString::from_str(env, path)?;
            env.call_method(
                activity,
                jni_str!("renderPoster"),
                jni_sig!("(Ljava/lang/String;Ljava/lang/String;)Z"),
                &[JValue::Object(&html), JValue::Object(&path)],
            )?
            .z()
        })
        .unwrap_or(false)
    }

    pub fn device_name(&self) -> Option<String> {
        self.call("deviceName", |env, activity| {
            let name = env
                .call_method(
                    activity,
                    jni_str!("deviceName"),
                    jni_sig!("()Ljava/lang/String;"),
                    &[],
                )?
                .l()?;
            let name = env.cast_local::<JString>(name)?;
            name.try_to_string(env)
        })
    }

    pub fn notifications_enabled(&self) -> bool {
        self.call("notificationsEnabled", |env, activity| {
            env.call_method(
                activity,
                jni_str!("notificationsEnabled"),
                jni_sig!("()Z"),
                &[],
            )?
            .z()
        })
        .unwrap_or(false)
    }

    pub fn request_notifications(&self) {
        self.call("requestNotifications", |env, activity| {
            env.call_method(
                activity,
                jni_str!("requestNotifications"),
                jni_sig!("()V"),
                &[],
            )?;
            Ok(())
        });
    }

    pub fn post_notification(&self, notification: &Notification) -> bool {
        self.call("postNotification", |env, activity| {
            let channel = JString::from_str(env, notification.channel.id)?;
            let channel_name = JString::from_str(env, notification.channel.name)?;
            let title = JString::from_str(env, &notification.title)?;
            let text = JString::from_str(env, &notification.text)?;
            let subtext = match &notification.subtext {
                Some(subtext) => JString::from_str(env, subtext)?.into(),
                None => JObject::null(),
            };
            let url = JString::from_str(env, &notification.url)?;
            let labels = string_array(env, notification.actions.iter().map(|(label, _)| label.as_str()))?;
            let urls = string_array(env, notification.actions.iter().map(|(_, url)| url.as_str()))?;
            env.call_method(
                activity,
                jni_str!("postNotification"),
                jni_sig!(
                    "(Ljava/lang/String;Ljava/lang/String;IILjava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;[Ljava/lang/String;[Ljava/lang/String;ZI)V"
                ),
                &[
                    JValue::Object(&channel),
                    JValue::Object(&channel_name),
                    JValue::Int(notification.channel.importance.android()),
                    JValue::Int(notification.id),
                    JValue::Object(&title),
                    JValue::Object(&text),
                    JValue::Object(&subtext),
                    JValue::Object(&url),
                    JValue::Object(&labels),
                    JValue::Object(&urls),
                    JValue::Bool(notification.ongoing),
                    JValue::Int((0xFF00_0000 | notification.color) as jint),
                ],
            )?;
            Ok(())
        })
        .is_some()
    }

    pub fn cancel_notification(&self, id: i32) {
        self.call("cancelNotification", |env, activity| {
            env.call_method(
                activity,
                jni_str!("cancelNotification"),
                jni_sig!("(I)V"),
                &[JValue::Int(id)],
            )?;
            Ok(())
        });
    }

    /// -1 follows night mode, 0 light icons, 1 dark icons.
    pub fn set_bar_icons(&self, mode: i32) {
        self.call("setBarIcons", |env, activity| {
            env.call_method(
                activity,
                jni_str!("setBarIcons"),
                jni_sig!("(I)V"),
                &[JValue::Int(mode)],
            )?;
            Ok(())
        });
    }

    pub fn long_press_feedback(&self) {
        self.call("longPressFeedback", |env, activity| {
            env.call_method(
                activity,
                jni_str!("longPressFeedback"),
                jni_sig!("()V"),
                &[],
            )?;
            Ok(())
        });
    }

    /// The display's fastest refresh rate at its current resolution, in hertz.
    pub fn max_refresh_rate(&self) -> Option<f32> {
        self.call("maxRefreshRate", |env, activity| {
            env.call_method(activity, jni_str!("maxRefreshRate"), jni_sig!("()F"), &[])?
                .f()
        })
        .filter(|rate| *rate > 0.)
    }
}

/// Runs a native method body, turning JNI errors and panics into a Java exception.
fn native(env: &mut EnvUnowned<'_>, f: impl FnOnce(&mut Env) -> jni::errors::Result<()>) {
    env.with_env(f)
        .resolve::<jni::errors::ThrowRuntimeExAndDefault>()
}

/// Android offsets are -1 for "none".
fn offset(value: jint) -> usize {
    value.max(0) as usize
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_pi_gpui_GpuiActivity_nativeImeState<'caller>(
    mut env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    seq: jint,
    push_id: jint,
    text: JString<'caller>,
    selection_start: jint,
    selection_end: jint,
    composing_start: jint,
    composing_end: jint,
) {
    native(&mut env, |env| {
        let text = text.try_to_string(env)?;
        let (start, end) = (offset(selection_start), offset(selection_end));
        deliver(JavaEvent::Ime {
            seq,
            push_id,
            state: ImeState {
                text,
                selection: start.min(end)..start.max(end),
                composing: (composing_start >= 0 && composing_start < composing_end)
                    .then(|| offset(composing_start)..offset(composing_end)),
            },
        });
        Ok(())
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_pi_gpui_GpuiActivity_nativePicked<'caller>(
    mut env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    request: jint,
    paths: JString<'caller>,
    error: JString<'caller>,
) {
    native(&mut env, |env| {
        let result = if !error.is_null() {
            Err(error.try_to_string(env)?)
        } else if paths.is_null() {
            Ok(None)
        } else {
            let paths = paths.try_to_string(env)?;
            Ok(Some(paths.split('\0').map(String::from).collect()))
        };
        deliver(JavaEvent::Picked { request, result });
        Ok(())
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_pi_gpui_GpuiActivity_nativeOpenUrl<'caller>(
    mut env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    url: JString<'caller>,
) {
    native(&mut env, |env| {
        deliver(JavaEvent::OpenUrl(url.try_to_string(env)?));
        Ok(())
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_pi_gpui_GpuiActivity_nativeEditorAction<'caller>(
    mut env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    action: jint,
) {
    native(&mut env, |_| {
        deliver(JavaEvent::EditorAction(action));
        Ok(())
    });
}

#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_dev_pi_gpui_GpuiActivity_nativeInsets<'caller>(
    mut env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    bars_left: jint,
    bars_top: jint,
    bars_right: jint,
    bars_bottom: jint,
    ime_left: jint,
    ime_top: jint,
    ime_right: jint,
    ime_bottom: jint,
) {
    native(&mut env, |_| {
        deliver(JavaEvent::Insets {
            bars: [bars_left, bars_top, bars_right, bars_bottom],
            ime: [ime_left, ime_top, ime_right, ime_bottom],
        });
        Ok(())
    });
}

/// Decodes the luminance plane from Camera2 without uploading or retaining a frame.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_pi_gpui_PairScannerActivity_nativeDecodeQr<'caller>(
    mut env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    bytes: JByteArray<'caller>,
    width: jint,
    height: jint,
    row_stride: jint,
) -> jstring {
    let mut result = std::ptr::null_mut();
    native(&mut env, |env| {
        let (width, height, stride) = (width as usize, height as usize, row_stride as usize);
        if width == 0 || height == 0 || width > 8192 || height > 8192 || stride < width {
            return Ok(());
        }
        let source = env.convert_byte_array(&bytes)?;
        let needed = stride.saturating_mul(height.saturating_sub(1)) + width;
        if source.len() < needed {
            return Ok(());
        }
        let mut image = if stride == width {
            source[..width * height].to_vec()
        } else {
            let mut packed = Vec::with_capacity(width * height);
            for row in 0..height {
                let start = row * stride;
                packed.extend_from_slice(&source[start..start + width]);
            }
            packed
        };
        if let Some(text) = crate::qr::decode(width, height, &mut image) {
            result = JString::from_str(env, &text)?.into_raw();
        }
        Ok(())
    });
    result
}

/// A `String[]` for Java.
fn string_array<'local, 'a>(
    env: &mut Env<'local>,
    items: impl ExactSizeIterator<Item = &'a str>,
) -> jni::errors::Result<JObjectArray<'local>> {
    let array = env.new_object_array(
        items.len() as jint,
        jni_str!("java/lang/String"),
        JObject::null(),
    )?;
    for (index, item) in items.enumerate() {
        let item = JString::from_str(env, item)?;
        array.set_element(env, index, item)?;
    }
    Ok(array)
}
