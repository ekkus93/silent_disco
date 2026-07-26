package com.ekkus.silentdisco.app

import kotlin.random.Random

const val INVITE_CODE_LENGTH = 4

fun generateInviteCode(
    nextDigit: () -> Int = { Random.nextInt(0, 10) },
): String = buildString(INVITE_CODE_LENGTH) {
    repeat(INVITE_CODE_LENGTH) {
        append(nextDigit().coerceIn(0, 9))
    }
}

fun normalizeInviteCode(value: String): String = value
    .filterNot(Char::isWhitespace)
    .trim()

fun inviteCodeValidationError(value: String): String? {
    val normalized = normalizeInviteCode(value)
    return when {
        normalized.isBlank() -> "Enter an invite code."
        normalized.length != INVITE_CODE_LENGTH -> "Invite codes must contain $INVITE_CODE_LENGTH digits."
        normalized.any { !it.isDigit() } -> "Invite codes can contain digits only."
        else -> null
    }
}

fun isValidInviteCode(value: String): Boolean = inviteCodeValidationError(value) == null
