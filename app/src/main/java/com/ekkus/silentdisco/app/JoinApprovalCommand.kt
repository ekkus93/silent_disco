package com.ekkus.silentdisco.app

import androidx.annotation.MainThread
import com.ekkus.silentdisco.core.model.JoinRequest

/** Dispatches an immutable per-request approval lifetime directly to Rust. */
@MainThread
internal fun MainViewModel.dispatchJoinApproval(
    request: JoinRequest,
    rememberForFuture: Boolean,
) {
    approveJoinRequest(request, rememberForFuture)
}
