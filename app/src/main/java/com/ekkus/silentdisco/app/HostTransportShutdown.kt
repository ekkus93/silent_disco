package com.ekkus.silentdisco.app

import java.io.Closeable
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.withContext

/**
 * Retire the native host transport off the UI thread before acknowledging
 * StopAdvertising to the Rust actor. Native shutdown can wait for an in-flight
 * FFI send and a blocking socket poll; cancellation must not skip teardown.
 */
internal suspend fun retireHostTransportOffMain(
    transport: AutoCloseable,
    dispatcher: CoroutineDispatcher = Dispatchers.IO,
) {
    withContext(NonCancellable + dispatcher) {
        transport.close()
    }
}
