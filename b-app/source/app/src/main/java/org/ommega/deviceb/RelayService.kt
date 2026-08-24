package org.ommega.deviceb

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import android.os.PowerManager
import android.util.Log
import java.lang.ref.WeakReference
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import org.json.JSONObject

class RelayService : Service() {
    private val TAG = "DeviceB.RelayService"
    private val CHANNEL_ID = "deviceb_relay"
    private val NOTIF_ID = 1
    private val POLL_TIMEOUT_SEC = 10
    // Mark disconnected after this idle period.
    private val DISCONNECT_THRESHOLD_MS = (POLL_TIMEOUT_SEC + 8) * 1000L
    private val RETRY_BASE_MS = 1200L
    private val RETRY_MAX_MS = 10_000L
    // Retry delay after 409 conflict.
    private val CONFLICT_RETRY_MS = 15_000L
    // Maximum alias length - matches Android AOSP keystore limit (KEY_SIZE = (NAME_MAX - 15) / 2 = 120)
    private val MAX_ALIAS_LENGTH = 120
    // Number of worker threads for task processing.
    // Keystore/TEE operations are typically serialized at the HAL level,
    // but a small pool helps overlap network I/O with TEE processing.
    private val TASK_THREADS = 4

    private var running = false
    private var lastPollSuccessMs = 0L
    private var keepWakeLock: PowerManager.WakeLock? = null
    private lateinit var pollThread: Thread
    private lateinit var taskExecutor: ExecutorService

    var connectionState: ConnectionState = ConnectionState.IDLE
        private set

    enum class ConnectionState { IDLE, CONNECTED, DISCONNECTED, CONFLICT }

    interface ConnectionListener {
        fun onStateChanged(state: ConnectionState)
    }
    var connectionListener: ConnectionListener? = null

    companion object {
        const val PREFS_NAME = "deviceb_prefs"

        /** Listener registered by MainActivity. */
        private var uiListenerRef: WeakReference<ConnectionListener>? = null

        var uiListener: ConnectionListener?
            get() = uiListenerRef?.get()
            set(value) {
                uiListenerRef = value?.let { WeakReference(it) }
            }
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        createNotificationChannel()
        val notif = buildNotification("⏳ Connecting -> ${ServerClient.serverUrl}")
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            startForeground(NOTIF_ID, notif, ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
        } else {
            startForeground(NOTIF_ID, notif)
        }
        acquireWakeLock()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (!running) {
            running = true
            lastPollSuccessMs = System.currentTimeMillis()
            taskExecutor = Executors.newFixedThreadPool(TASK_THREADS) { r ->
                Thread(r, "relay-task").apply { isDaemon = true }
            }
            pollThread = Thread(::pollLoop, "relay-poll").apply { isDaemon = true; start() }
        }
        // Refresh client config on each start command.
        applyLatestConfig()
        return START_STICKY
    }

    private fun applyLatestConfig() {
        val prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE)
        val url = prefs.getString("server_url", ServerClient.serverUrl)!!.trim().trimEnd('/')
        val deviceId = prefs.getString("device_id", ServerClient.deviceId)!!.trim()
        val token = prefs.getString("relay_token", ServerClient.relayToken)!!.trim()
        val tlsInsecure = prefs.getBoolean("tls_insecure", false)
        if (url != ServerClient.serverUrl ||
            deviceId != ServerClient.deviceId ||
            token != ServerClient.relayToken ||
            tlsInsecure != ServerClient.tlsInsecure
        ) {
            ServerClient.serverUrl = url
            ServerClient.deviceId = deviceId
            ServerClient.relayToken = token
            ServerClient.tlsInsecure = tlsInsecure
            Log.i(TAG, "Config updated: url=$url deviceId=$deviceId tls_insecure=$tlsInsecure")
            updateNotification("⏳ Connecting -> $url")
        }
    }

    override fun onDestroy() {
        running = false
        updateConnectionState(ConnectionState.IDLE)
        if (::pollThread.isInitialized) pollThread.interrupt()
        if (::taskExecutor.isInitialized) {
            taskExecutor.shutdown()
            try {
                taskExecutor.awaitTermination(5, TimeUnit.SECONDS)
            } catch (_: InterruptedException) { }
        }
        keepWakeLock?.let { if (it.isHeld) it.release() }
        keepWakeLock = null
        super.onDestroy()
    }

    private fun acquireWakeLock() {
        if (keepWakeLock?.isHeld == true) return
        val pm = getSystemService(POWER_SERVICE) as? PowerManager ?: return
        keepWakeLock = pm.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "$packageName:relay_worker").apply {
            setReferenceCounted(false)
            acquire()
        }
        Log.i(TAG, "PARTIAL_WAKE_LOCK acquired")
    }

    private fun pollLoop() {
        Log.i(TAG, "Worker started -> ${ServerClient.serverUrl}")
        var retryDelayMs = RETRY_BASE_MS
        while (running && !Thread.currentThread().isInterrupted) {
            try {
                applyLatestConfig()
                val task = ServerClient.pollTask(timeoutSec = POLL_TIMEOUT_SEC)
                // 200/204 both count as healthy connectivity.
                lastPollSuccessMs = System.currentTimeMillis()
                updateConnectionState(ConnectionState.CONNECTED)
                retryDelayMs = RETRY_BASE_MS
                if (task != null) {
                    // Offload task execution to the thread pool so that slow
                    // operations (key generation, RSA decrypt of large blobs)
                    // do not block the poll loop from fetching the next task.
                    taskExecutor.submit { processTask(task) }
                }
            } catch (e: ServerClient.ConflictException) {
                // 409: device_id is currently owned elsewhere.
                Log.w(TAG, "pollLoop: 409 conflict -> ${e.message}")
                updateConnectionState(ConnectionState.CONFLICT)
                // Backoff and retry.
                try { Thread.sleep(CONFLICT_RETRY_MS) } catch (_: InterruptedException) { break }
            } catch (e: InterruptedException) {
                Thread.currentThread().interrupt(); break
            } catch (e: Exception) {
                Log.e(TAG, "pollLoop: ${e.message}")
                val elapsed = System.currentTimeMillis() - lastPollSuccessMs
                if (elapsed > DISCONNECT_THRESHOLD_MS) {
                    updateConnectionState(ConnectionState.DISCONNECTED)
                }
                try { Thread.sleep(retryDelayMs) } catch (_: InterruptedException) { break }
                retryDelayMs = (retryDelayMs * 2).coerceAtMost(RETRY_MAX_MS)
            }
        }
        Log.i(TAG, "Worker stopped")
    }

    private fun updateConnectionState(newState: ConnectionState) {
        if (connectionState == newState) return
        connectionState = newState
        connectionListener?.onStateChanged(newState)
        uiListener?.onStateChanged(newState)
        val text = when (newState) {
            ConnectionState.CONNECTED    -> "✅ Connected -> ${ServerClient.serverUrl}"
            ConnectionState.DISCONNECTED -> "❌ Disconnected -> ${ServerClient.serverUrl}"
            ConnectionState.CONFLICT     -> "⚠️ Conflict: device_id already in use"
            ConnectionState.IDLE         -> "⏳ Connecting -> ${ServerClient.serverUrl}"
        }
        updateNotification(text)
        Log.i(TAG, "Connection state: $newState")
    }

    private fun updateNotification(text: String) {
        val nm = getSystemService(NotificationManager::class.java)
        nm.notify(NOTIF_ID, buildNotification(text))
    }

    private fun processTask(task: JSONObject) {
        if (!task.has("task_id") || !task.has("task_type") || !task.has("payload")) {
            Log.w(TAG, "ignore invalid task payload: $task")
            return
        }
        val taskId = task.getString("task_id")
        val taskType = task.getString("task_type")
        val payload = task.getJSONObject("payload")
        val alias = payload.optString("alias", "").ifEmpty { "deviceb_attest_key" }

        // Validate alias length to prevent DoS attacks
        if (alias.toByteArray(Charsets.UTF_8).size > MAX_ALIAS_LENGTH) {
            Log.w(TAG, "reject task $taskId: alias exceeds $MAX_ALIAS_LENGTH bytes (${alias.toByteArray(Charsets.UTF_8).size})")
            ServerClient.postResult(taskId, JSONObject().apply {
                put("error", "keystore_error:7")
                put("error_code", 7)
                put("message", "alias too long (max $MAX_ALIAS_LENGTH bytes)")
            })
            return
        }

        // System-only: every task is fulfilled through the real Android
        // Keystore / TEE (no custom keybox). The attestation application ID
        // is the system's own (this app's) — it cannot be overridden.
        // The A-side's requested key parameters (algorithm / curve / purposes /
        // digests / paddings / certificate fields) are honored when present,
        // otherwise the defaults (EC P-256 + SHA-256 + SIGN) are used.
        val spec = TaskKeySpec.fromPayload(payload)

        // StrongBox（securityLevel==2）请求：直接交给 Android Keystore 原生
        // 行为处理 —— 有真实 StrongBox 芯片则出 StrongBox 链；无 StrongBox 时
        // setIsStrongBoxBacked 的原生行为是静默降级为 TEE（链如实标记
        // TRUSTED_ENVIRONMENT），这也是 Android 的标准行为，照常放行；原生
        // 抛异常（部分 ROM）则如实上报，由 server 三层回退 / A 端本地 keybox
        // 兜底。本端不做额外策略，结果与真实设备表现一致（b-side 二进制 relay
        // 直连真实 StrongBox HAL 才需要 HAL 语义的严格处理）。

        val result = try {
            when (taskType) {
                "attest" ->
                    KeystoreHelper.attest(this, payload.getString("challenge"), alias, spec)
                "sign" ->
                    KeystoreHelper.sign(
                        this,
                        payload.getString("data"),
                        alias,
                        payload.optString("algorithm", "SHA256withECDSA"),
                    )
                "decrypt" ->
                    KeystoreHelper.decrypt(
                        this,
                        payload.getString("data"),
                        alias,
                        payload.optString("algorithm", "RSA/ECB/PKCS1Padding"),
                    )
                else -> JSONObject().put("error", "unknown: $taskType")
            }
        } catch (e: Exception) {
            JSONObject().put("error", e.message ?: "error")
        }
        val ok = ServerClient.postResult(taskId, result)
        if (!ok) Log.w(TAG, "postResult failed taskId=$taskId type=$taskType")
    }

    private fun createNotificationChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val ch = NotificationChannel(CHANNEL_ID, "DeviceB Relay", NotificationManager.IMPORTANCE_LOW)
            getSystemService(NotificationManager::class.java).createNotificationChannel(ch)
        }
    }

    private fun buildNotification(text: String): Notification {
        val b = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O)
            Notification.Builder(this, CHANNEL_ID)
        else @Suppress("DEPRECATION") Notification.Builder(this)
        return b.setContentTitle("DeviceB Relay")
            .setContentText(text)
            .setSmallIcon(android.R.drawable.ic_dialog_info)
            .setOngoing(true).build()
    }
}
