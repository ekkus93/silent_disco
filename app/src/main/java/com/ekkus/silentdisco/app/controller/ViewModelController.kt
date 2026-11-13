package com.ekkus.silentdisco.app.controller

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel

/**
 * One responsibility split out of `MainViewModel` (God-file refactor spec
 * §4). A controller owns its own state and background work; nothing else
 * writes that state.
 *
 * Its [scope] is a child of the view-model's scope. Clearing the view-model
 * cancels it, and so does [close]. A failing job does not cancel its
 * siblings, as with `viewModelScope`.
 */
internal abstract class ViewModelController(parentScope: CoroutineScope) : AutoCloseable {
    protected val scope: CoroutineScope = CoroutineScope(
        parentScope.coroutineContext + SupervisorJob(parentScope.coroutineContext[Job]),
    )

    /** Releases what [scope] cannot: native handles, executors, files. */
    protected open fun release() = Unit

    /** Cancels this controller's work, then releases its resources. */
    final override fun close() {
        scope.cancel()
        release()
    }
}

/**
 * The controllers one view-model owns, closed in reverse creation order (a
 * controller may depend on one created before it). Every controller is
 * closed even if an earlier close fails; the first failure is rethrown with
 * the others suppressed.
 */
internal class ViewModelControllers {
    private val controllers = mutableListOf<AutoCloseable>()

    /** Registers [controller] for [closeAll] and returns it. */
    fun <T : AutoCloseable> add(controller: T): T {
        controllers += controller
        return controller
    }

    fun closeAll() {
        var first: Throwable? = null
        for (controller in controllers.asReversed()) {
            try {
                controller.close()
            } catch (error: Throwable) {
                val earlier = first
                if (earlier == null) first = error else earlier.addSuppressed(error)
            }
        }
        controllers.clear()
        first?.let { throw it }
    }
}
