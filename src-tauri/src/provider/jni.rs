//! The two functions Kotlin calls (`scripts/android/ProviderNative.kt`). Everything else is [`Provider::call`].
//!
//! * `init(dataDir)` (the directory from `Context.getDataDir()`) opens the process's core (or finds the one the window opened) and builds the provider.
//! * `call(op, args)` runs one operation and returns the JSON answer.
//!
//! Both are static methods of the app's own class, so only the app's own code can reach them. A panic or a JNI error never
//! crosses into Java as anything but an exception message; the JSON answer of an operation never carries a secret.

use super::{shared, Provider};
use jni::objects::{JClass, JString};
use jni::EnvUnowned;
use std::path::PathBuf;
use std::sync::OnceLock;

static PROVIDER: OnceLock<Provider> = OnceLock::new();

fn failure(code: &str, message: &str) -> String {
    serde_json::json!({"ok": false, "code": code, "message": message}).to_string()
}

fn init(data_dir: &str) -> String {
    if PROVIDER.get().is_some() {
        return serde_json::json!({"ok": true, "data": {}}).to_string();
    }
    // `data_dir` is the per-app directory Android gives (`Context.getDataDir()`); the window keeps its files in the folder
    // named after the app's identifier inside it (`app_data_dir`), and so does this.
    match shared::get_or_open(&PathBuf::from(data_dir).join(super::app_identifier())) {
        Ok(core) => {
            let _ = PROVIDER.set(Provider::new(core));
            serde_json::json!({"ok": true, "data": {}}).to_string()
        }
        Err(e) => failure(e.code(), &e.to_string()),
    }
}

fn call(op: &str, args: &str) -> String {
    match PROVIDER.get() {
        Some(p) => p.call(op, args),
        None => failure("internal", "The passkey service was not started."),
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_habibullah_hexis_provider_ProviderNative_init<'caller>(
    mut env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    data_dir: JString<'caller>,
) -> JString<'caller> {
    let outcome = env.with_env(|env| -> Result<_, jni::errors::Error> {
        let dir = data_dir.try_to_string(env)?;
        JString::from_str(env, init(&dir))
    });
    outcome.resolve::<jni::errors::ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_habibullah_hexis_provider_ProviderNative_call<'caller>(
    mut env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    op: JString<'caller>,
    args: JString<'caller>,
) -> JString<'caller> {
    let outcome = env.with_env(|env| -> Result<_, jni::errors::Error> {
        let op = op.try_to_string(env)?;
        let args = args.try_to_string(env)?;
        JString::from_str(env, call(&op, &args))
    });
    outcome.resolve::<jni::errors::ThrowRuntimeExAndDefault>()
}
