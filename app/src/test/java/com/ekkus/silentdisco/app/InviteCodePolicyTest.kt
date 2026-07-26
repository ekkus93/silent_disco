package com.ekkus.silentdisco.app

import com.google.common.truth.Truth.assertThat
import org.junit.Test

class InviteCodePolicyTest {
    @Test
    fun generatorProducesFourReadableDigits() {
        val digits = ArrayDeque(listOf(4, 8, 2, 7))

        val code = generateInviteCode { digits.removeFirst() }

        assertThat(code).isEqualTo("4827")
    }

    @Test
    fun whitespaceIsNormalizedBeforeValidation() {
        assertThat(normalizeInviteCode(" 48 27 ")).isEqualTo("4827")
        assertThat(inviteCodeValidationError(" 48 27 ")).isNull()
    }

    @Test
    fun invalidLengthAndCharactersAreRejected() {
        assertThat(inviteCodeValidationError("123")).contains("4 digits")
        assertThat(inviteCodeValidationError("12A4")).contains("digits only")
    }
}
