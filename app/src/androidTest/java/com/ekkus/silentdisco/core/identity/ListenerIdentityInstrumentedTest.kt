package com.ekkus.silentdisco.core.identity

import java.security.AlgorithmParameters
import java.security.KeyFactory
import java.security.KeyStore
import java.security.Signature
import java.security.interfaces.ECPublicKey
import java.security.spec.ECGenParameterSpec
import java.security.spec.ECParameterSpec
import java.security.spec.X509EncodedKeySpec
import java.util.UUID
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Real Android Keystore: the listener proof key is P-256, non-exportable, stable and separate from the host key. */
class ListenerIdentityInstrumentedTest {
    private val aliases = mutableListOf<String>()
    private val keyStore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }

    private fun freshAlias(): String = "silent-disco-listener-test-${UUID.randomUUID()}".also { aliases += it }

    @After
    fun deleteTestKeys() {
        aliases.forEach { runCatching { keyStore.deleteEntry(it) } }
    }

    private fun verify(publicKeyDer: ByteArray, payload: ByteArray, signature: ByteArray): Boolean =
        Signature.getInstance("SHA256withECDSA").run {
            initVerify(KeyFactory.getInstance("EC").generatePublic(X509EncodedKeySpec(publicKeyDer)))
            update(payload)
            verify(signature)
        }

    @Test
    fun theProofKeyIsP256AndItsSignaturesVerifyAgainstItsPublicKey() {
        val manager = ListenerIdentityManager(freshAlias())
        val spki = manager.publicKeyDer()

        val ecKey = KeyFactory.getInstance("EC").generatePublic(X509EncodedKeySpec(spki)) as ECPublicKey
        val p256 = AlgorithmParameters.getInstance("EC").run {
            init(ECGenParameterSpec("secp256r1"))
            getParameterSpec(ECParameterSpec::class.java)
        }
        assertEquals(p256.curve, ecKey.params.curve)
        assertEquals(p256.generator, ecKey.params.generator)

        val payload = "listener proof transcript".toByteArray()
        val signature = manager.signDer(payload)
        assertEquals("the signature must be DER, which the core verifier expects", 0x30, signature[0].toInt())
        assertTrue(verify(spki, payload, signature))
        assertTrue(!verify(spki, "another transcript".toByteArray(), signature))
    }

    @Test
    fun theSameAliasKeepsItsIdentityAcrossManagerInstances() {
        val alias = freshAlias()
        val first = ListenerIdentityManager(alias).publicKeyDer()
        val second = ListenerIdentityManager(alias).publicKeyDer()
        assertArrayEquals(first, second)
    }

    @Test
    fun theListenerKeyIsSeparateFromTheHostKey() {
        val listener = ListenerIdentityManager(freshAlias()).publicKeyDer()
        val host = HostIdentityManager(alias = freshAlias()).publicKeyDer()
        assertNotEquals(listener.toList(), host.toList())
    }

    @Test
    fun thePrivateKeyCannotBeExported() {
        val alias = freshAlias()
        ListenerIdentityManager(alias).publicKeyDer()
        val entry = keyStore.getEntry(alias, null) as java.security.KeyStore.PrivateKeyEntry
        assertNull("a Keystore private key must not expose encoded bytes", entry.privateKey.encoded)
    }
}
