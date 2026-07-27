package com.ekkus.silentdisco.app

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.viewModels
import com.ekkus.silentdisco.feature.settings.TrustedDevicesViewModel
import com.ekkus.silentdisco.ui.theme.SilentDiscoTheme

class MainActivity : ComponentActivity() {
    private val mainViewModel by viewModels<MainViewModel>()
    private val workflowViewModel by viewModels<WorkflowViewModel>()
    private val trustedDevicesViewModel by viewModels<TrustedDevicesViewModel>()
    private val p2ViewModel by viewModels<P2ViewModel>()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent {
            SilentDiscoTheme {
                SilentDiscoApp(
                    viewModel = mainViewModel,
                    workflowViewModel = workflowViewModel,
                    trustedDevicesViewModel = trustedDevicesViewModel,
                    p2ViewModel = p2ViewModel,
                )
            }
        }
    }
}
