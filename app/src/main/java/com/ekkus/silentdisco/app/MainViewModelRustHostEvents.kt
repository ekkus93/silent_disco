package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.rust.HostCoreController
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportEvent
import com.ekkus.silentdisco.core.uniffi.FfiHostTransportHandle

/** Native events are projected into the Rust actor with their source generation; see `HostSessionController`. */
internal fun MainViewModel.handleHostTransportEvent(
    controller: HostCoreController,
    event: FfiHostTransportEvent,
    sourceHandle: FfiHostTransportHandle,
) = hostSession.handleTransportEvent(controller, event, sourceHandle)
