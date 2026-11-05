package com.ekkus.silentdisco.core.transport

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.runTest
import org.junit.Test
import org.mockito.ArgumentCaptor
import org.mockito.Mockito

/**
 * Drives the real service's NSD discovery listener with an announcement flood.
 * NSD resolves one service at a time, so everything beyond the in-flight
 * resolve queues; the queue is finite and the overflow is visible exactly once.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class MdnsDiscoveryServiceFloodTest {
    private fun announced(name: String): NsdServiceInfo {
        val info = Mockito.mock(NsdServiceInfo::class.java)
        Mockito.`when`(info.serviceName).thenReturn(name)
        Mockito.`when`(info.serviceType).thenReturn("_silentdisco._tcp.")
        return info
    }

    @Test
    fun announcementFloodResolvesOneAtATimeQueuesAtMostTheCapAndReportsFullOnce() =
        runTest(UnconfinedTestDispatcher()) {
            val manager = Mockito.mock(NsdManager::class.java)
            val context = Mockito.mock(Context::class.java)
            Mockito.`when`(context.applicationContext).thenReturn(context)
            Mockito.`when`(context.getSystemService(NsdManager::class.java)).thenReturn(manager)
            val service = MdnsDiscoveryService(context)
            val failures = mutableListOf<String>()
            val collector: Job = launch { service.failures.collect { failures += it } }

            assertThat(service.startDiscovery().started).isTrue()
            val captor = ArgumentCaptor.forClass(NsdManager.DiscoveryListener::class.java)
            Mockito.verify(manager).discoverServices(
                Mockito.anyString(), Mockito.eq(NsdManager.PROTOCOL_DNS_SD), captor.capture(),
            )
            val listener = captor.value

            repeat(1_000) { listener.onServiceFound(announced("flood-$it")) }

            // One resolve is in flight; the rest wait in a queue capped at the limit.
            Mockito.verify(manager, Mockito.times(1)).resolveService(
                Mockito.any(NsdServiceInfo::class.java), Mockito.any(NsdManager.ResolveListener::class.java),
            )
            val full = failures.filter { it.contains("list is full") }
            assertThat(full).hasSize(1)
            assertThat(failures.size).isEqualTo(1)

            collector.cancel()
        }
}
