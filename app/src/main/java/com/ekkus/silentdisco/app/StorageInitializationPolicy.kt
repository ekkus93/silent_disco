package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.rust.RustDatabaseException

enum class StorageInitializationState {
    INITIALIZING,
    READY,
    RECOVERABLE_FAILURE,
    FATAL_FAILURE,
}

internal data class StorageFailurePresentation(
    val state: StorageInitializationState,
    val message: String,
)

private val recoverableDatabaseStatuses = setOf(
    -105, // SQLite busy or worker queue full.
    -110, // Database worker temporarily unavailable.
)

internal fun classifyStorageInitializationFailure(error: Throwable): StorageFailurePresentation {
    val causes = generateSequence(error) { current -> current.cause }
        .take(16)
        .toList()
    val databaseError = causes.filterIsInstance<RustDatabaseException>().firstOrNull()
    val state = if (databaseError?.statusCode in recoverableDatabaseStatuses) {
        StorageInitializationState.RECOVERABLE_FAILURE
    } else {
        StorageInitializationState.FATAL_FAILURE
    }
    val detail = databaseError?.message
        ?: error.message?.takeIf(String::isNotBlank)
        ?: error.javaClass.simpleName
    val prefix = when (state) {
        StorageInitializationState.RECOVERABLE_FAILURE ->
            "Persistent storage is temporarily unavailable"
        StorageInitializationState.FATAL_FAILURE ->
            "Fatal persistent storage failure"
        StorageInitializationState.INITIALIZING,
        StorageInitializationState.READY ->
            error("A successful storage state cannot describe an initialization failure")
    }
    return StorageFailurePresentation(
        state = state,
        message = "$prefix: $detail",
    )
}

internal fun AppUiState.withStorageInitializationFailure(error: Throwable): AppUiState {
    val presentation = classifyStorageInitializationFailure(error)
    return copy(
        storageState = presentation.state,
        storageError = presentation.message,
        lastMessage = null,
        lastError = presentation.message,
    )
}

internal fun AppUiState.storageStatusLabel(): String = when (storageState) {
    StorageInitializationState.INITIALIZING -> "Initializing Rust storage…"
    StorageInitializationState.READY -> "Rust storage ready"
    StorageInitializationState.RECOVERABLE_FAILURE -> "Storage temporarily unavailable"
    StorageInitializationState.FATAL_FAILURE -> "Storage initialization failed"
}

internal fun AppUiState.canRetryStorageInitialization(): Boolean =
    storageState == StorageInitializationState.RECOVERABLE_FAILURE

internal fun AppUiState.persistentFeaturesEnabled(): Boolean =
    storageState == StorageInitializationState.READY
