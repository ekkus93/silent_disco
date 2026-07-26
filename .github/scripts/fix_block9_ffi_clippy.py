from pathlib import Path

path = Path("rust/silent-disco-ffi/src/android_storage.rs")
text = path.read_text(encoding="utf-8")
old_registry = '''    let mut registry = match storage_registry().lock() {
        Ok(registry) => registry,
        Err(_) => {
            let shutdown = worker.stop_and_join().map_err(BridgeFailure::from);
            return shutdown.and(Err(BridgeFailure::registry_poisoned("open_storage")));
        }
    };'''
new_registry = '''    let Ok(mut registry) = storage_registry().lock() else {
        let shutdown = worker.stop_and_join().map_err(BridgeFailure::from);
        return shutdown.and(Err(BridgeFailure::registry_poisoned("open_storage")));
    };'''
if text.count(old_registry) != 1:
    raise RuntimeError(f"registry match count was {text.count(old_registry)}")
text = text.replace(old_registry, new_registry)
old_raw = ".map_or(core::ptr::null_mut(), |value| value.into_raw())"
new_raw = ".map_or(core::ptr::null_mut(), jni::objects::JString::into_raw)"
if text.count(old_raw) != 1:
    raise RuntimeError(f"JNI raw conversion count was {text.count(old_raw)}")
path.write_text(text.replace(old_raw, new_raw), encoding="utf-8")
