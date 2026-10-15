//! Owned JavaVM/global reference: safe to call from a credential worker, never a UI loop.
//! No Activity or JNI local reference survives construction. Errors are deliberately redacted.
use jni::{
    Env, JValue, JavaVM, jni_sig, jni_str,
    objects::{JObject, JString},
    refs::Global,
};

pub struct ProviderVault {
    vm: JavaVM,
    object: Global<JObject<'static>>,
}
impl ProviderVault {
    pub(crate) fn from_object(env: &mut Env, object: JObject<'_>) -> jni::errors::Result<Self> {
        Ok(Self {
            vm: env.get_java_vm()?,
            object: env.new_global_ref(object)?,
        })
    }
    pub fn read(&self, provider: &str) -> Result<String, &'static str> {
        self.vm
            .attach_current_thread(|env| -> jni::errors::Result<String> {
                let provider = env.new_string(provider)?;
                let result = env
                    .call_method(
                        self.object.as_ref(),
                        jni_str!("read"),
                        jni_sig!("(Ljava/lang/String;)Ljava/lang/String;"),
                        &[JValue::Object(&provider)],
                    )?
                    .l()?;
                env.cast_local::<JString>(result)?.try_to_string(env)
            })
            .map_err(|_| "credential storage unavailable")
    }
    pub fn write(
        &self,
        provider: &str,
        expected: &str,
        credential: &str,
    ) -> Result<Option<String>, &'static str> {
        self.vm.attach_current_thread(|env| -> jni::errors::Result<Option<String>> {
            let provider = env.new_string(provider)?; let expected = env.new_string(expected)?; let credential = env.new_string(credential)?;
            let result = env.call_method(self.object.as_ref(), jni_str!("write"), jni_sig!("(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;"),
                &[JValue::Object(&provider), JValue::Object(&expected), JValue::Object(&credential)])?.l()?;
            if result.is_null() { Ok(None) } else { Ok(Some(env.cast_local::<JString>(result)?.try_to_string(env)?)) }
        }).map_err(|_| "credential save failed")
    }
    pub fn clear(&self, provider: &str) -> Result<(), &'static str> {
        self.vm
            .attach_current_thread(|env| -> jni::errors::Result<()> {
                let provider = env.new_string(provider)?;
                env.call_method(
                    self.object.as_ref(),
                    jni_str!("clear"),
                    jni_sig!("(Ljava/lang/String;)V"),
                    &[JValue::Object(&provider)],
                )?;
                Ok(())
            })
            .map_err(|_| "credential removal failed")
    }
}

/// Main-thread construction only; subsequent I/O must be on a worker.
pub fn provider_vault(synthetic: bool) -> Option<ProviderVault> {
    crate::java::current()?.provider_vault(synthetic)
}
