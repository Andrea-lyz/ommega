package org.ommega.deviceb

import android.app.Application
import android.provider.Settings
import android.util.Base64
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.net.URI
import java.security.SecureRandom
import java.util.concurrent.Executors

data class RelayConfig(val url: String = "", val deviceId: String = "", val token: String = "", val tlsInsecure: Boolean = true)
internal fun normalizeRelayConfig(input: RelayConfig): RelayConfig? {
    val config = input.copy(url = input.url.trim().trimEnd('/'), deviceId = input.deviceId.trim(), token = input.token.trim())
    val url = runCatching { URI(config.url) }.getOrNull() ?: return null
    return config.takeIf {
        url.scheme in listOf("http", "https") && !url.host.isNullOrEmpty() && url.userInfo == null && url.query == null && url.fragment == null &&
            config.deviceId.isNotBlank() && config.deviceId.none(Char::isWhitespace) &&
            config.token.isNotBlank() && config.token.all { it.code in 33..126 }
    }
}
data class RelayUiState(
    val config: RelayConfig = RelayConfig(),
    val connection: RelayService.ConnectionState? = null,
    val message: Int = R.string.ready,
    val busy: Boolean = false,
    val diagnosticsBusy: Boolean = false,
    val diagnostics: String = "",
    val diagnosticCount: Int? = null,
    val diagnosticFailed: Boolean = false,
    val theme: String = "auto",
)

class RelayViewModel(application: Application) : AndroidViewModel(application) {
    private val prefs = application.getSharedPreferences(RelayService.PREFS_NAME, 0)
    private val mutableState = MutableStateFlow(RelayUiState(
        config = RelayConfig(prefs.getString("server_url", "") ?: "", prefs.getString("device_id", "") ?: "",
            prefs.getString("relay_token", "") ?: "", prefs.getBoolean("tls_insecure", true)),
        theme = prefs.getString("ui_theme", "auto") ?: "auto",
    ))
    val state = mutableState.asStateFlow()
    // A submitted real Keystore call must finish without cancellation or interruption.
    private val diagnosticWorker = Executors.newSingleThreadExecutor()
    fun edit(config: RelayConfig) { mutableState.update { it.copy(config = config) } }
    fun connectionChanged(value: RelayService.ConnectionState) { mutableState.update { it.copy(connection = value) } }
    fun message(value: Int) { mutableState.update { it.copy(message = value) } }
    fun theme(value: String) {
        prefs.edit().putString("ui_theme", value).apply()
        mutableState.update { it.copy(theme = value) }
    }
    fun saveAndApply(): Boolean {
        val config = normalizeRelayConfig(state.value.config)
        if (config == null) {
            message(R.string.invalid_configuration); return false
        }
        prefs.edit().putString("server_url", config.url).putString("device_id", config.deviceId)
            .putString("relay_token", config.token).putBoolean("tls_insecure", config.tlsInsecure).apply()
        ServerClient.serverUrl = config.url
        ServerClient.deviceId = config.deviceId
        ServerClient.relayToken = config.token
        ServerClient.tlsInsecure = config.tlsInsecure
        ServerClient.machineId = {
            Settings.Secure.getString(getApplication<Application>().contentResolver, Settings.Secure.ANDROID_ID) ?: ""
        }
        mutableState.update { it.copy(config = config, message = R.string.saved) }
        return true
    }
    fun ping() {
        if (state.value.busy) return
        val config = normalizeRelayConfig(state.value.config)
        if (config == null) { message(R.string.invalid_configuration); return }
        mutableState.update { it.copy(busy = true, message = R.string.checking) }
        viewModelScope.launch {
            val ok = withContext(Dispatchers.IO) { runCatching { ServerClient.ping(config.url, config.token, config.tlsInsecure) }.getOrDefault(false) }
            mutableState.update { it.copy(busy = false, message = if (ok) R.string.ping_ok else R.string.ping_failed) }
        }
    }
    fun diagnose(attest: Boolean) {
        if (state.value.diagnosticsBusy) return
        mutableState.update { it.copy(diagnosticsBusy = true, diagnostics = "", diagnosticCount = null, diagnosticFailed = false) }
        diagnosticWorker.execute {
            val app = getApplication<Application>()
            val result = runCatching {
                val json = if (attest) {
                    val challenge = ByteArray(32).also { SecureRandom().nextBytes(it) }
                    KeystoreHelper.attest(app, Base64.encodeToString(challenge, Base64.NO_WRAP))
                } else KeystoreHelper.getCertChain(app)
                val chain = json.getJSONArray("cert_chain")
                chain.length() to buildString {
                    for (i in 0 until chain.length()) appendLine("[$i] ${chain.getString(i).take(60)}…")
                }
            }
            mutableState.update { it.copy(diagnosticsBusy = false, diagnostics = result.getOrNull()?.second ?: "", diagnosticCount = result.getOrNull()?.first, diagnosticFailed = result.isFailure) }
        }
    }
    override fun onCleared() {
        diagnosticWorker.shutdown() // Let a submitted TEE operation finish. Never shutdownNow().
        super.onCleared()
    }
}
