package app.yawrec.mobile

import android.app.Application
import androidx.lifecycle.AndroidViewModel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import org.json.JSONObject
import java.util.concurrent.TimeUnit

// ── Modèles ───────────────────────────────────────────────────────────────────

data class DesktopStatus(
    val phase: String = "idle",        // "idle" | "recording" | "paused"
    val elapsed: String = "00:00:00",
    val sizeHuman: String = "0 B",
    val sizeBytes: Long = 0L,
    val frameCount: Long = 0L,
)

sealed class ConnectionState {
    object Disconnected : ConnectionState()
    object Connecting : ConnectionState()
    object Connected : ConnectionState()
    data class Failed(val message: String) : ConnectionState()
}

// ── ViewModel ────────────────────────────────────────────────────────────────

private const val PREFS_NAME = "remote_prefs"
private const val KEY_LAST_IP = "last_ip"

class RemoteViewModel(app: Application) : AndroidViewModel(app) {

    private val prefs = app.getSharedPreferences(PREFS_NAME, 0)

    private val _ip = MutableStateFlow(prefs.getString(KEY_LAST_IP, "") ?: "")
    val ip: StateFlow<String> = _ip.asStateFlow()

    private val _connState = MutableStateFlow<ConnectionState>(ConnectionState.Disconnected)
    val connState: StateFlow<ConnectionState> = _connState.asStateFlow()

    private val _desktopStatus = MutableStateFlow(DesktopStatus())
    val desktopStatus: StateFlow<DesktopStatus> = _desktopStatus.asStateFlow()

    private val _stoppedPath = MutableStateFlow<String?>(null)
    val stoppedPath: StateFlow<String?> = _stoppedPath.asStateFlow()

    private var webSocket: WebSocket? = null

    private val httpClient = OkHttpClient.Builder()
        .connectTimeout(5, TimeUnit.SECONDS)
        .readTimeout(0, TimeUnit.SECONDS)  // connexion persistante, pas de timeout lecture
        .build()

    fun onIpChanged(value: String) { _ip.value = value }

    fun fillIpAndConnect(ip: String) {
        _ip.value = ip
        connect()
    }

    private var lastConnectedHost = ""

    fun connect() {
        val host = _ip.value.trim()
        if (host.isEmpty()) return
        disconnect()
        lastConnectedHost = host
        _connState.value = ConnectionState.Connecting
        _stoppedPath.value = null
        val req = Request.Builder().url("ws://$host:9799").build()
        webSocket = httpClient.newWebSocket(req, Listener())
    }

    fun disconnect() {
        webSocket?.close(1000, null)
        webSocket = null
        _connState.value = ConnectionState.Disconnected
        _desktopStatus.value = DesktopStatus()
    }

    fun send(cmd: String) {
        webSocket?.send("""{"cmd":"$cmd"}""")
    }

    override fun onCleared() {
        disconnect()
        httpClient.dispatcher.executorService.shutdown()
        super.onCleared()
    }

    // ── WebSocket listener ────────────────────────────────────────────────────

    private inner class Listener : WebSocketListener() {

        override fun onOpen(webSocket: WebSocket, response: Response) {
            _connState.value = ConnectionState.Connected
            prefs.edit().putString(KEY_LAST_IP, lastConnectedHost).apply()
        }

        override fun onMessage(webSocket: WebSocket, text: String) {
            try {
                val json = JSONObject(text)
                when (json.getString("event")) {
                    "status" -> _desktopStatus.value = DesktopStatus(
                        phase      = json.getString("phase"),
                        elapsed    = json.getString("elapsed"),
                        sizeHuman  = json.getString("size_human"),
                        sizeBytes  = json.getLong("size_bytes"),
                        frameCount = json.getLong("frame_count"),
                    )
                    "stopped" -> {
                        _stoppedPath.value = json.optString("path", "")
                        _desktopStatus.value = DesktopStatus()
                    }
                    "error" -> {
                        // les erreurs de commande ne déconnectent pas
                        android.util.Log.w("YawREC-WS", json.optString("message", "error"))
                    }
                }
            } catch (_: Exception) { /* message malformé, ignorer */ }
        }

        override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
            webSocket.close(1000, null)
            _connState.value = ConnectionState.Disconnected
            _desktopStatus.value = DesktopStatus()
        }

        override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
            _connState.value = ConnectionState.Failed(t.message ?: "Connexion échouée")
            _desktopStatus.value = DesktopStatus()
        }
    }
}
