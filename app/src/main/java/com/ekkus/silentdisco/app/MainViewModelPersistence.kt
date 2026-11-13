package com.ekkus.silentdisco.app


    internal fun MainViewModel.initializeDomainPersistence() {
        _uiState.value = _uiState.value.copy(
            storageState = StorageInitializationState.INITIALIZING,
            storageError = null,
            lastMessage = "Initializing persistent storage",
            lastError = null,
        )
        persistence.initialize { result ->
            result.onSuccess { stored ->
                val tuning = stored.toAppTuningSettings()
                _uiState.value = _uiState.value.copy(
                    tuningSettings = tuning,
                    storageState = StorageInitializationState.READY,
                    storageError = null,
                    lastMessage = "Persistent settings loaded",
                    lastError = null,
                )
            }.onFailure { error ->
                logger.e("storage.initialize", error.message.orEmpty(), error)
                _uiState.value = _uiState.value.withStorageInitializationFailure(error)
            }
        }
    }

    internal fun MainViewModel.requirePersistenceReady(action: String): Boolean {
        if (_uiState.value.persistentFeaturesEnabled()) return true
        val message = "${_uiState.value.storageStatusLabel()}; cannot $action."
        _uiState.value = _uiState.value.copy(lastError = message)
        return false
    }
