#!/usr/bin/env python3
"""Apply the audited desktop Block 10 compiler and fail-visibility repairs."""

from pathlib import Path
import re


def replace_pattern(path: str, pattern: str, replacement: str, marker: str) -> None:
    target = Path(path)
    source = target.read_text()
    updated, count = re.subn(
        pattern,
        replacement,
        source,
        count=1,
        flags=re.MULTILINE | re.DOTALL,
    )
    if count == 1:
        target.write_text(updated)
        return
    if marker in source:
        return
    raise SystemExit(f"expected source was not found in {path}: {marker}")


notification = "desktop/src-tauri/src/notification_buffer.rs"
replace_pattern(
    notification,
    r'''return Err\(CoreError \{\s*code: CoreErrorCode::QueueOverflow,\s*message: "desktop pending notification queue is full"\.to_owned\(\),\s*subsystem: CoreErrorCode::QueueOverflow\.subsystem\(\),\s*severity: ErrorSeverity::Fatal,\s*retryable: false,\s*operation_id: None,\s*context: Vec::new\(\),\s*\}\);''',
    '''return Err(core_error(
                        CoreErrorCode::QueueOverflow,
                        "desktop pending notification queue is full",
                    ));''',
    "desktop pending notification queue is full\",\n                    ));",
)
replace_pattern(
    notification,
    r'''fn bridge_error\(message: &str\) -> CoreError \{\s*CoreError \{\s*code: CoreErrorCode::FfiCallbackFailed,\s*message: message\.to_owned\(\),\s*subsystem: CoreErrorCode::FfiCallbackFailed\.subsystem\(\),\s*severity: ErrorSeverity::Fatal,\s*retryable: false,\s*operation_id: None,\s*context: Vec::new\(\),\s*\}\s*\}''',
    '''fn bridge_error(message: &'static str) -> CoreError {
    core_error(CoreErrorCode::FfiCallbackFailed, message)
}

fn core_error(code: CoreErrorCode, message: &'static str) -> CoreError {
    match CoreError::new(code, message, ErrorSeverity::Fatal, false, None) {
        Ok(error) => error,
        Err(error) => panic!("invalid static desktop notification error: {error}"),
    }
}''',
    "fn core_error(code: CoreErrorCode",
)
replace_pattern(
    notification,
    r'''CoreNotification::Error\(CoreError \{\s*code: CoreErrorCode::PlatformOperationFailed,\s*message: format!\("failure \{index\}"\),\s*subsystem: CoreErrorCode::PlatformOperationFailed\.subsystem\(\),\s*severity: ErrorSeverity::Error,\s*retryable: false,\s*operation_id: None,\s*context: Vec::new\(\),\s*\}\)''',
    '''CoreNotification::Error(
                    CoreError::new(
                        CoreErrorCode::PlatformOperationFailed,
                        format!("failure {index}"),
                        ErrorSeverity::Error,
                        false,
                        None,
                    )
                    .expect("valid test error"),
                )''',
    'format!("failure {index}"),\n                        ErrorSeverity::Error',
)
replace_pattern(
    notification,
    r'''CoreNotification::Error\(CoreError \{\s*code: CoreErrorCode::PlatformOperationFailed,\s*message: "overflow"\.to_owned\(\),\s*subsystem: CoreErrorCode::PlatformOperationFailed\.subsystem\(\),\s*severity: ErrorSeverity::Error,\s*retryable: false,\s*operation_id: None,\s*context: Vec::new\(\),\s*\}\)''',
    '''CoreNotification::Error(
                CoreError::new(
                    CoreErrorCode::PlatformOperationFailed,
                    "overflow",
                    ErrorSeverity::Error,
                    false,
                    None,
                )
                .expect("valid test error"),
            )''',
    '"overflow",\n                    ErrorSeverity::Error',
)

replace_pattern(
    "desktop/src-tauri/src/platform/identity.rs",
    r'''Self::GenerateSecret\(source\) => Some\(source\),\s*Self::InvalidStoredSecretLength \{ \.\. \}\s*\| Self::IdentifierEncodingFailed\s*\| Self::DerivedIdentifierInvalid => None,''',
    '''Self::GenerateSecret(_)
            | Self::InvalidStoredSecretLength { .. }
            | Self::IdentifierEncodingFailed
            | Self::DerivedIdentifierInvalid => None,''',
    "Self::GenerateSecret(_)",
)

app_state = "desktop/src-tauri/src/app_state.rs"
replace_pattern(
    app_state,
    r'''identity: DesktopIdentity,\s*handle: CoreActorHandle,\s*notifications: Arc<DesktopNotificationBuffer>,''',
    '''_identity: DesktopIdentity,
    handle: CoreActorHandle,
    _notifications: Arc<DesktopNotificationBuffer>,''',
    "_identity: DesktopIdentity",
)
replace_pattern(
    app_state,
    r'''let cleanup = shutdown_owned_resources\(ready\.owned\);\s*let error = append_cleanup\(primary, cleanup\.err\(\)\);\s*let _ = self\.fail_open\(error\.clone\(\)\);\s*Err\(error\)''',
    '''let cleanup = shutdown_owned_resources(ready.owned);
                    let error = append_cleanup(primary, cleanup.err());
                    if let Err(state_error) = self.fail_open(error.clone()) {
                        return Err(append_cleanup(error, Some(state_error)));
                    }
                    Err(error)''',
    "if let Err(state_error) = self.fail_open",
)
replace_pattern(
    app_state,
    r'''CloseAction::Shutdown\(ready\) => \{\s*let _public_identity = ready\.identity\.device_id\(\);\s*let _pending_bridge = Arc::strong_count\(&ready\.notifications\);\s*self\.finish_close\(shutdown_owned_resources\(ready\.owned\)\)\s*\}''',
    '''CloseAction::Shutdown(ready) => {
                self.finish_close(shutdown_owned_resources(ready.owned))
            }''',
    "CloseAction::Shutdown(ready) => {\n                self.finish_close",
)
replace_pattern(
    app_state,
    r'''\n            identity,\s*handle,\s*notifications,\n''',
    '''
            _identity: identity,
            handle,
            _notifications: notifications,
''',
    "_notifications: notifications",
)

replace_pattern(
    "desktop/src-tauri/src/runtime_dto.rs",
    r'''fn saturating_u32\(value: usize\) -> u32 \{\s*u32::try_from\(value\)\.unwrap_or\(u32::MAX\)\s*\}''',
    '''fn saturating_u32(value: usize) -> u32 {
    match u32::try_from(value) {
        Ok(value) => value,
        Err(error) => panic!("validated core collection count exceeded u32: {error}"),
    }
}''',
    "validated core collection count exceeded u32",
)
