package org.ommega.deviceb

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.activity.viewModels
import androidx.appcompat.app.AppCompatActivity
import androidx.appcompat.app.AppCompatDelegate
import androidx.compose.runtime.getValue
import androidx.compose.runtime.SideEffect
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.core.content.ContextCompat
import androidx.core.os.LocaleListCompat
import androidx.core.view.WindowCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.lifecycleScope
import androidx.lifecycle.repeatOnLifecycle
import kotlinx.coroutines.launch

class MainActivity : AppCompatActivity() {
    private val model: RelayViewModel by viewModels()
    private val notificationPermission = registerForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        if (granted) startRelay() else model.message(R.string.notification_required)
    }
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        if (!BuildConfig.UI_PREVIEW) lifecycleScope.launch {
            repeatOnLifecycle(Lifecycle.State.STARTED) {
                RelayService.connectionUpdates.collect { value -> value?.let(model::connectionChanged) }
            }
        }
        setContent {
            val observed by model.state.collectAsStateWithLifecycle()
            val state = if (BuildConfig.UI_PREVIEW) observed.copy(
                config = RelayConfig("https://relay.example.com:8443", "dedicated-b", "", false),
                connection = RelayService.ConnectionState.CONNECTED,
            ) else observed
            val dark = when (state.theme) { "dark" -> true; "light" -> false; else -> isSystemInDarkTheme() }
            SideEffect {
                WindowCompat.getInsetsController(window, window.decorView).apply {
                    isAppearanceLightStatusBars = !dark
                    isAppearanceLightNavigationBars = !dark
                }
            }
            OmmegaTheme(state.theme) {
                RelayScreen(state = state, onConfigChange = model::edit,
                    onAction = { action -> if (!BuildConfig.UI_PREVIEW) handleAction(action) }, onThemeChange = model::theme,
                    onLanguageChange = { language -> AppCompatDelegate.setApplicationLocales(LocaleListCompat.forLanguageTags(if (language == "auto") "" else language)) },
                    preview = BuildConfig.UI_PREVIEW)
            }
        }
    }
    private fun handleAction(action: RelayAction) {
        when (action) {
            RelayAction.Start -> {
                if (!model.saveAndApply()) return
                if (Build.VERSION.SDK_INT >= 33 && ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
                    notificationPermission.launch(Manifest.permission.POST_NOTIFICATIONS)
                } else startRelay()
            }
            RelayAction.Stop -> { stopService(Intent(this, RelayService::class.java)); model.message(R.string.stopped) }
            RelayAction.Ping -> model.ping()
            RelayAction.Attest -> model.diagnose(true)
            RelayAction.Certificates -> model.diagnose(false)
        }
    }
    private fun startRelay() {
        ContextCompat.startForegroundService(this, Intent(this, RelayService::class.java))
        model.message(R.string.service_started)
    }
}
