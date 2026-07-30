from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TARGET = ROOT / "rust/silent-disco-core/src/runtime/actor_runtime/mod.rs"
content = TARGET.read_text(encoding="utf-8")
old = '''        let handle = runtime.handle();
        for _ in 0..100 {
            if handle.current_snapshot().is_err() {
                break;
            }
            std::thread::yield_now();
        }
        assert!(handle.current_snapshot().is_err());
        assert!(runtime.shutdown().is_err());
'''
new = '''        let handle = runtime.handle();
        let shutdown_error = runtime
            .shutdown()
            .expect_err("observer failure must make controlled shutdown fail visibly");
        assert_eq!(shutdown_error.code, CoreErrorCode::ShutdownFailed);
        let observer_error = handle
            .current_snapshot()
            .expect_err("joined observer failure must remain visible through the handle");
        assert_eq!(observer_error.code, CoreErrorCode::FfiCallbackFailed);
'''
if content.count(old) != 1:
    raise RuntimeError("expected one observer-failure race block")
TARGET.write_text(content.replace(old, new, 1), encoding="utf-8")
(ROOT / "scripts/fix-block14-observer-failure-test.py").unlink()
