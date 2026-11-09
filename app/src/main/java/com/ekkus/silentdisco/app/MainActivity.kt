package com.ekkus.silentdisco.app

import android.content.IntentFilter
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.viewModels
import androidx.core.content.ContextCompat
import com.ekkus.silentdisco.BuildConfig
import com.ekkus.silentdisco.feature.settings.TrustedDevicesViewModel
import com.ekkus.silentdisco.ui.theme.SilentDiscoTheme

class MainActivity : ComponentActivity() {
    private val mainViewModel by viewModels<MainViewModel>()
    private val workflowViewModel by viewModels<WorkflowViewModel>()
    private val trustedDevicesViewModel by viewModels<TrustedDevicesViewModel>()
    private val p2ViewModel by viewModels<P2ViewModel>()
    private var debugCaptureReceiver: DebugCaptureReceiver? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        if (BuildConfig.DEBUG) {
            // Debug builds only: lets a developer script start/stop the listener PCM capture.
            val receiver = DebugCaptureReceiver(mainViewModel)
            ContextCompat.registerReceiver(
                this,
                receiver,
                IntentFilter(DebugCaptureReceiver.ACTION),
                ContextCompat.RECEIVER_EXPORTED,
            )
            debugCaptureReceiver = receiver
        }
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

    override fun onDestroy() {
        debugCaptureReceiver?.let(::unregisterReceiver)
        debugCaptureReceiver = null
        super.onDestroy()
    }
}
