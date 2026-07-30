#!/usr/bin/env python3
"""Apply compile- and test-safe fixups after the Block 13 Android cutover."""

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def replace_once(path: str, old: str, new: str) -> None:
    target = ROOT / path
    content = target.read_text(encoding="utf-8")
    count = content.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected one match, found {count}")
    target.write_text(content.replace(old, new), encoding="utf-8")


def normalize_eof(path: str) -> None:
    target = ROOT / path
    content = target.read_text(encoding="utf-8")
    target.write_text(content.rstrip() + "\n", encoding="utf-8")


def remove_stale_test(path: str) -> None:
    target = ROOT / path
    if not target.exists():
        raise SystemExit(f"{path}: expected stale Kotlin authority test")
    target.unlink()


def write_invite_code_test() -> None:
    path = ROOT / "app/src/test/java/com/ekkus/silentdisco/app/InviteCodeValidationTest.kt"
    path.write_text(
        '''package com.ekkus.silentdisco.app

import android.net.Uri
import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.model.SelectedAudioFile
import com.ekkus.silentdisco.core.uniffi.FfiApprovalMode
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test
import org.mockito.kotlin.mock
import org.mockito.kotlin.whenever

class InviteCodeValidationTest {
    @Test
    fun inviteCodeAndOpaqueAudioIdentityArePassedToRustWithoutKotlinValidation() {
        val uri: Uri = mock()
        whenever(uri.toString()).thenReturn("content://private/audio/42")
        val draft = HostFormState(
            sessionName = "Invite session",
            approvalMode = ApprovalMode.INVITE_CODE,
            inviteCode = " 2468 ",
            selectedAudio = SelectedAudioFile(
                uri = uri,
                displayName = "set.wav",
                mimeType = "audio/wav",
                sizeBytes = 512,
            ),
        ).toFfiHostDraft(TuningSettings())

        assertEquals(FfiApprovalMode.INVITE_CODE, draft.approvalMode)
        assertEquals(" 2468 ", draft.inviteCode)
        assertFalse(draft.audioSource!!.sourceId.contains("content://"))
    }
}
''',
        encoding="utf-8",
    )


def harden_tcp_server_shutdown() -> None:
    path = "app/src/main/java/com/ekkus/silentdisco/core/transport/TcpTransport.kt"
    replace_once(
        path,
        "import java.net.Socket\n",
        "import java.net.Socket\nimport java.net.SocketException\n",
    )
    replace_once(
        path,
        '''        acceptJob = scope.launch {
            while (true) {
                val socket = serverSocket?.accept() ?: break
                val remoteAddress = socket.remoteSocketAddress.toString()
''',
        '''        acceptJob = scope.launch {
            while (true) {
                val socket = try {
                    serverSocket?.accept() ?: break
                } catch (error: SocketException) {
                    if (serverSocket?.isClosed == true) break
                    throw error
                }
                val remoteAddress = socket.remoteSocketAddress.toString()
''',
    )


def main() -> None:
    replace_once(
        "app/src/main/java/com/ekkus/silentdisco/core/rust/HostCoreController.kt",
        "        handle.createHostSession(snapshot.revision)\n    }\n",
        "        handle.createHostSession(snapshot.revision)\n        Unit\n    }\n",
    )
    normalize_eof("app/src/main/java/com/ekkus/silentdisco/app/AppState.kt")
    normalize_eof("app/src/main/java/com/ekkus/silentdisco/app/MainViewModelHostActions.kt")
    remove_stale_test(
        "app/src/test/java/com/ekkus/silentdisco/app/HostStartupValidationTest.kt",
    )
    write_invite_code_test()
    harden_tcp_server_shutdown()


if __name__ == "__main__":
    main()
