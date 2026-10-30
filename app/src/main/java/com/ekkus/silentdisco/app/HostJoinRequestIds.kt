package com.ekkus.silentdisco.app

import java.util.UUID

/**
 * Identity of a single observed host-side join attempt, not of the device.
 * Legacy v3 does not authenticate the claimed device ID; reconnecting with
 * the same claim must never reuse a prior actor approval request ID.
 */
internal fun newHostJoinRequestId(): String = UUID.randomUUID().toString()
