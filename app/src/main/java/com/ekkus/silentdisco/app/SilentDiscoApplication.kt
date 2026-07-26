package com.ekkus.silentdisco.app

import android.app.Application
import com.ekkus.silentdisco.core.persistence.AndroidStorageRepository

class SilentDiscoApplication : Application() {
    val storageRepository: AndroidStorageRepository by lazy(LazyThreadSafetyMode.SYNCHRONIZED) {
        AndroidStorageRepository(this)
    }
}
