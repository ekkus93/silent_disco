package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.model.TrustState

internal data class JoinApprovalExecutionResult(
    val delivered: Boolean,
    val trustedForFuture: Boolean,
    val persistenceError: Throwable?,
) {
    val trustState: TrustState
        get() = if (trustedForFuture) TrustState.TRUSTED_PLACEHOLDER else TrustState.SESSION_ONLY
}

internal suspend fun executeJoinApproval(
    rememberForFuture: Boolean,
    persistTrust: suspend () -> Result<Unit>,
    sendApproval: suspend (trustedForFuture: Boolean) -> Boolean,
): JoinApprovalExecutionResult {
    val persistenceResult = if (rememberForFuture) persistTrust() else Result.success(Unit)
    val trustedForFuture = rememberForFuture && persistenceResult.isSuccess
    return JoinApprovalExecutionResult(
        delivered = sendApproval(trustedForFuture),
        trustedForFuture = trustedForFuture,
        persistenceError = persistenceResult.exceptionOrNull(),
    )
}
