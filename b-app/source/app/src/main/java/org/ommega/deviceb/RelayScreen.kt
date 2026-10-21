package org.ommega.deviceb

import androidx.activity.compose.BackHandler
import androidx.appcompat.app.AppCompatDelegate
import androidx.compose.animation.AnimatedContent
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Home
import androidx.compose.material.icons.filled.Info
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.tooling.preview.Preview

enum class RelayAction { Start, Stop, Ping, Attest, Certificates }

private val DarkColors = darkColorScheme(primary = Color(0xFF8DE5CD), onPrimary = Color(0xFF063A30),
    secondaryContainer = Color(0xFF17473E), onSecondaryContainer = Color(0xFF8DE5CD),
    primaryContainer = Color(0xFF17473E), onPrimaryContainer = Color(0xFF8DE5CD),
    background = Color(0xFF10181C), onBackground = Color(0xFFE9F0F2), surface = Color(0xFF19252A),
    onSurface = Color(0xFFE9F0F2), surfaceVariant = Color(0xFF223138), onSurfaceVariant = Color(0xFFA8BAC2))
private val LightColors = lightColorScheme(primary = Color(0xFF006A60), onPrimary = Color.White,
    secondaryContainer = Color(0xFFD4F4E9), onSecondaryContainer = Color(0xFF005146),
    primaryContainer = Color(0xFFD4F4E9), onPrimaryContainer = Color(0xFF005146),
    background = Color(0xFFF5F7F8), onBackground = Color(0xFF172B32), surface = Color.White,
    onSurface = Color(0xFF172B32), surfaceVariant = Color(0xFFEEF2F3), onSurfaceVariant = Color(0xFF52676E))

@Composable
fun OmmegaTheme(preference: String, content: @Composable () -> Unit) {
    val dark = when (preference) { "dark" -> true; "light" -> false; else -> isSystemInDarkTheme() }
    MaterialTheme(colorScheme = if (dark) DarkColors else LightColors, content = content)
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun RelayScreen(
    state: RelayUiState,
    onConfigChange: (RelayConfig) -> Unit,
    onAction: (RelayAction) -> Unit,
    onThemeChange: (String) -> Unit,
    onLanguageChange: (String) -> Unit,
    modifier: Modifier = Modifier,
    preview: Boolean = false,
) {
    var tab by rememberSaveable { mutableIntStateOf(0) }
    val holder = rememberSaveableStateHolder()
    BackHandler(enabled = tab != 0) { tab = 0 }
    val titles = listOf(R.string.overview, R.string.connection, R.string.settings)
    val icons = listOf(Icons.Default.Home, Icons.Default.Settings, Icons.Default.Info)
    Scaffold(
        modifier = modifier,
        containerColor = MaterialTheme.colorScheme.background,
        topBar = { TopAppBar(title = {
            Column { Text("Ommega B", fontWeight = FontWeight.Bold); Text("ATTESTATION / PROVIDER", style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant) }
        }, colors = TopAppBarDefaults.topAppBarColors(containerColor = MaterialTheme.colorScheme.background)) },
        bottomBar = { NavigationBar(containerColor = MaterialTheme.colorScheme.surface) {
            titles.forEachIndexed { index, title -> NavigationBarItem(selected = tab == index, onClick = { tab = index },
                icon = { Icon(icons[index], contentDescription = null) }, label = { Text(stringResource(title)) }) }
        } },
    ) { padding ->
        AnimatedContent(targetState = tab, modifier = Modifier.padding(padding), label = "main-destination") { selected ->
            holder.SaveableStateProvider(selected) {
                LazyColumn(modifier = Modifier.fillMaxSize().padding(horizontal = 20.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
                    if (preview) item("preview") { Text(stringResource(R.string.preview_note), color = MaterialTheme.colorScheme.primary, modifier = Modifier.padding(vertical = 12.dp)) }
                    when (selected) {
                        0 -> {
                            item("hero") { OmmegaCard(stringResource(R.string.hardware_ready)) {
                                ConnectionBadge(state.connection)
                                Text(stringResource(R.string.hero_description), color = MaterialTheme.colorScheme.onSurfaceVariant)
                                HorizontalDivider()
                                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                                    Text("A", fontWeight = FontWeight.Bold); Text("→"); Text("Server"); Text("→"); Text("B · TEE", fontWeight = FontWeight.Bold)
                                }
                            } }
                            item("device") { OmmegaCard(stringResource(R.string.device_id)) { Text(state.config.deviceId.ifBlank { "—" }, fontSize = 24.sp, fontWeight = FontWeight.SemiBold) } }
                            item("server") { OmmegaCard(stringResource(R.string.server_url)) {
                                Text(publicServerUrl(state.config.url).ifBlank { "—" })
                                Text(stringResource(R.string.status_source), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                            } }
                            item("notice") { Notice(state) }
                        }
                        1 -> {
                            item("form") { ConnectionForm(state.config, onConfigChange, preview) }
                            item("actions") { Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                                Button(onClick = { onAction(RelayAction.Start) }, enabled = !preview && !state.diagnosticsBusy, modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)) { Text(stringResource(R.string.start_service)) }
                                OutlinedButton(onClick = { onAction(RelayAction.Ping) }, enabled = !preview && !state.busy, modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)) { Text(stringResource(R.string.check_connection)) }
                                StopServiceButton(enabled = !preview && !state.diagnosticsBusy, onConfirm = { onAction(RelayAction.Stop) })
                            } }
                            item("notice") { Notice(state) }
                        }
                        2 -> {
                            item("appearance") { OmmegaCard(stringResource(R.string.appearance)) {
                                ChoiceRow(state.theme, onThemeChange)
                            } }
                            item("language") { OmmegaCard(stringResource(R.string.language)) {
                                val language = AppCompatDelegate.getApplicationLocales().toLanguageTags().ifEmpty { "auto" }
                                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                                    listOf("auto" to stringResource(R.string.system), "zh-CN" to "中文", "en" to "English").forEach { (value, label) ->
                                        FilterChip(selected = value == language, onClick = { onLanguageChange(value) }, label = { Text(label) })
                                    }
                                }
                            } }
                            item("diagnostics") { Diagnostics(state, onAction, preview) }
                        }
                    }
                    item("end-space") { Text("OMMEGA · ${BuildConfig.VERSION_NAME}", modifier = Modifier.padding(vertical = 16.dp), color = MaterialTheme.colorScheme.onSurfaceVariant, style = MaterialTheme.typography.labelSmall) }
                }
            }
        }
    }
}

// Public overview never echoes embedded credentials or query parameters.
internal fun publicServerUrl(raw: String): String = runCatching {
    val url = java.net.URI(raw)
    java.net.URI(url.scheme, null, url.host, url.port, url.path, null, null).toString()
}.getOrDefault("")

@Composable
private fun ConnectionBadge(connection: RelayService.ConnectionState?, modifier: Modifier = Modifier) {
    val title = when (connection) {
        RelayService.ConnectionState.CONNECTED -> R.string.connected
        RelayService.ConnectionState.DISCONNECTED -> R.string.disconnected
        RelayService.ConnectionState.CONFLICT -> R.string.conflict
        RelayService.ConnectionState.IDLE -> R.string.idle
        null -> R.string.not_reported
    }
    Surface(modifier = modifier, color = if (connection == RelayService.ConnectionState.CONNECTED) MaterialTheme.colorScheme.primaryContainer else MaterialTheme.colorScheme.surfaceVariant, shape = RoundedCornerShape(50)) {
        Text(stringResource(title), modifier = Modifier.padding(horizontal = 12.dp, vertical = 8.dp), style = MaterialTheme.typography.labelLarge)
    }
}

@Composable
private fun OmmegaCard(title: String, modifier: Modifier = Modifier, content: @Composable ColumnScope.() -> Unit) {
    Card(modifier = modifier.fillMaxWidth(), shape = RoundedCornerShape(24.dp), colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surface)) {
        Column(Modifier.padding(22.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
            Text(title, style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.SemiBold)
            content()
        }
    }
}

@Composable
private fun Notice(state: RelayUiState, modifier: Modifier = Modifier) {
    Column(modifier, verticalArrangement = Arrangement.spacedBy(12.dp)) {
        if (state.busy) LinearProgressIndicator(Modifier.fillMaxWidth())
        Text(stringResource(state.message), color = MaterialTheme.colorScheme.onSurfaceVariant, style = MaterialTheme.typography.bodyMedium)
    }
}

@Composable
private fun ConnectionForm(config: RelayConfig, onChange: (RelayConfig) -> Unit, preview: Boolean, modifier: Modifier = Modifier) {
    var showToken by remember { mutableStateOf(false) }
    OmmegaCard(stringResource(R.string.remote_connection), modifier) {
        OutlinedTextField(value = config.url, onValueChange = { onChange(config.copy(url = it)) }, label = { Text(stringResource(R.string.server_url)) }, singleLine = true, enabled = !preview, modifier = Modifier.fillMaxWidth())
        OutlinedTextField(value = config.deviceId, onValueChange = { onChange(config.copy(deviceId = it)) }, label = { Text(stringResource(R.string.device_id)) }, singleLine = true, enabled = !preview, modifier = Modifier.fillMaxWidth())
        OutlinedTextField(value = config.token, onValueChange = { onChange(config.copy(token = it)) }, label = { Text("Token") }, singleLine = true, enabled = !preview,
            visualTransformation = if (showToken) VisualTransformation.None else PasswordVisualTransformation(),
            trailingIcon = { TextButton(onClick = { showToken = !showToken }) { Text(stringResource(if (showToken) R.string.hide else R.string.show)) } }, modifier = Modifier.fillMaxWidth())
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
            Text(stringResource(R.string.trust_self_signed), Modifier.weight(1f).padding(end = 12.dp), style = MaterialTheme.typography.bodyMedium)
            Switch(checked = config.tlsInsecure, onCheckedChange = { onChange(config.copy(tlsInsecure = it)) }, enabled = !preview)
        }
    }
}

@Composable
private fun ChoiceRow(selected: String, onSelect: (String) -> Unit, modifier: Modifier = Modifier) {
    Row(modifier, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        listOf("auto" to R.string.system, "light" to R.string.light, "dark" to R.string.dark).forEach { (value, label) ->
            FilterChip(selected = selected == value, onClick = { onSelect(value) }, label = { Text(stringResource(label)) })
        }
    }
}

@Composable
private fun StopServiceButton(enabled: Boolean, onConfirm: () -> Unit, modifier: Modifier = Modifier) {
    var confirming by rememberSaveable { mutableStateOf(false) }
    TextButton(onClick = { confirming = true }, enabled = enabled, modifier = modifier.fillMaxWidth().heightIn(min = 48.dp)) { Text(stringResource(R.string.stop_service), color = MaterialTheme.colorScheme.error) }
    if (confirming) AlertDialog(onDismissRequest = { confirming = false }, title = { Text(stringResource(R.string.stop_service)) },
        text = { Text(stringResource(R.string.stop_confirmation)) },
        confirmButton = { TextButton(onClick = { confirming = false; onConfirm() }) { Text(stringResource(R.string.confirm)) } },
        dismissButton = { TextButton(onClick = { confirming = false }) { Text(stringResource(R.string.cancel)) } })
}

@Composable
private fun Diagnostics(state: RelayUiState, onAction: (RelayAction) -> Unit, preview: Boolean, modifier: Modifier = Modifier) {
    var expanded by rememberSaveable { mutableStateOf(false) }
    var pending by remember { mutableStateOf<RelayAction?>(null) }
    OmmegaCard(stringResource(R.string.diagnostics), modifier) {
        Text(stringResource(R.string.diagnostics_note), color = MaterialTheme.colorScheme.onSurfaceVariant, style = MaterialTheme.typography.bodySmall)
        TextButton(onClick = { expanded = !expanded }) { Text(stringResource(if (expanded) R.string.collapse else R.string.expand)) }
        if (expanded) {
            OutlinedButton(onClick = { pending = RelayAction.Attest }, enabled = !preview && !state.diagnosticsBusy, modifier = Modifier.fillMaxWidth()) { Text(stringResource(R.string.attest_test)) }
            OutlinedButton(onClick = { pending = RelayAction.Certificates }, enabled = !preview && !state.diagnosticsBusy, modifier = Modifier.fillMaxWidth()) { Text(stringResource(R.string.get_chain)) }
            if (state.diagnosticsBusy) LinearProgressIndicator(Modifier.fillMaxWidth())
            state.diagnosticCount?.let { Text(stringResource(R.string.certificate_count, it)) }
            if (state.diagnosticFailed) Text(stringResource(R.string.diagnostic_failed), color = MaterialTheme.colorScheme.error)
            if (state.diagnostics.isNotEmpty()) Text(state.diagnostics, fontFamily = FontFamily.Monospace, style = MaterialTheme.typography.bodySmall)
        }
    }
    if (pending != null) AlertDialog(onDismissRequest = { pending = null }, title = { Text(stringResource(R.string.diagnostics)) },
        text = { Text(stringResource(R.string.diagnostics_confirmation)) },
        confirmButton = { TextButton(onClick = { pending?.let(onAction); pending = null }) { Text(stringResource(R.string.confirm)) } },
        dismissButton = { TextButton(onClick = { pending = null }) { Text(stringResource(R.string.cancel)) } })
}

@Preview(showBackground = true, widthDp = 393)
@Composable
private fun RelayScreenPreview() {
    OmmegaTheme("dark") { RelayScreen(RelayUiState(config = RelayConfig("https://relay.example.com:8443", "dedicated-b"), connection = RelayService.ConnectionState.CONNECTED), {}, {}, {}, {}, preview = true) }
}
