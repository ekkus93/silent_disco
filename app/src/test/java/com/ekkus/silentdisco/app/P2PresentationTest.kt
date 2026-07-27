package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.rust.P2TrustedHost
import com.ekkus.silentdisco.core.rust.P2ValidatedInvitation
import com.google.common.truth.Truth.assertThat
import org.junit.Test

class P2PresentationTest {
    @Test
    fun approvalModesUseStableQrWireNames() {
        assertThat(ApprovalMode.MANUAL.qrWireName()).isEqualTo("manual")
        assertThat(ApprovalMode.INVITE_CODE.qrWireName()).isEqualTo("invite_code")
        assertThat(ApprovalMode.TRUSTED_DEVICES_PLACEHOLDER.qrWireName())
            .isEqualTo("approved_devices")
    }

    @Test
    fun validatedInvitationMapsOnlyRustApprovedModes() {
        val invitation = invitation(approvalMode = "invite_code", inviteCode = "4826")

        val session = invitation.toSessionInfo()

        assertThat(session.id).isEqualTo(invitation.sessionId)
        assertThat(session.hostDeviceName).isEqualTo(invitation.hostName)
        assertThat(session.approvalMode).isEqualTo(ApprovalMode.INVITE_CODE)
        assertThat(session.inviteCodeRequired).isTrue()
    }

    @Test
    fun trustRequiresMatchingFingerprintAndPublicKey() {
        val key = byteArrayOf(1, 2, 3)
        val state = P2UiState(
            validatedInvitation = invitation(
                fingerprint = "a".repeat(64),
                publicKey = key,
            ),
            trustedHosts = listOf(
                P2TrustedHost(
                    fingerprint = "a".repeat(64),
                    displayName = "Same visible name",
                    publicKeyDer = byteArrayOf(9, 9, 9),
                    lastVerifiedMs = 1L,
                ),
            ),
        )

        assertThat(state.validatedHostIsTrusted()).isFalse()
        assertThat(
            state.copy(
                trustedHosts = listOf(
                    P2TrustedHost(
                        fingerprint = "a".repeat(64),
                        displayName = "Different visible name",
                        publicKeyDer = key,
                        lastVerifiedMs = 1L,
                    ),
                ),
            ).validatedHostIsTrusted(),
        ).isTrue()
    }

    private fun invitation(
        approvalMode: String = "manual",
        inviteCode: String? = null,
        fingerprint: String = "a".repeat(64),
        publicKey: ByteArray = byteArrayOf(1, 2, 3),
    ) = P2ValidatedInvitation(
        sessionId = "session-1",
        sessionName = "Rooftop Disco",
        hostName = "Host phone",
        hostFingerprint = fingerprint,
        hostPublicKeyDer = publicKey,
        approvalMode = approvalMode,
        inviteCode = inviteCode,
        issuedAtMs = 1L,
        expiresAtMs = 2L,
    )
}
