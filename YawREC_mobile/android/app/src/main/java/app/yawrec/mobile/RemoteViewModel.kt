package app.yawrec.mobile

import android.app.Application
import android.net.Uri
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
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
    /** `retrying` : une reconnexion automatique est programmée. */
    data class Failed(val message: String, val retrying: Boolean = false) : ConnectionState()
}

/** Contenu d'un QR code YawREC : yawrec://<ip>:<port>?token=<code> */
data class PairingInfo(val host: String, val port: Int, val token: String)

fun parsePairingUri(raw: String): PairingInfo? {
    val uri = Uri.parse(raw.trim())
    if (uri.scheme != "yawrec") return null
    val host = uri.host?.takeIf { it.isNotBlank() } ?: return null
    val port = if (uri.port > 0) uri.port else DEFAULT_PORT
    return PairingInfo(host, port, uri.getQueryParameter("token").orEmpty())
}

// ── ViewModel ────────────────────────────────────────────────────────────────

private const val PREFS_NAME = "remote_prefs"
private const val KEY_LAST_IP = "last_ip"
private const val KEY_LAST_PORT = "last_port"
private const val KEY_TOKEN = "token"
private const val DEFAULT_PORT = 9799
private const val HTTP_UNAUTHORIZED = 401

class RemoteViewModel(app: Application) : AndroidViewModel(app) {

    private val prefs = app.getSharedPreferences(PREFS_NAME, 0)

    private val _ip = MutableStateFlow(prefs.getString(KEY_LAST_IP, "") ?: "")
    val ip: StateFlow<String> = _ip.asStateFlow()

    /** Code d'appairage affiché sous le QR code du bureau. */
    private val _token = MutableStateFlow(prefs.getString(KEY_TOKEN, "") ?: "")
    val token: StateFlow<String> = _token.asStateFlow()

    private var port = prefs.getInt(KEY_LAST_PORT, DEFAULT_PORT)

    private val _connState = MutableStateFlow<ConnectionState>(ConnectionState.Disconnected)
    val connState: StateFlow<ConnectionState> = _connState.asStateFlow()

    private val _desktopStatus = MutableStateFlow(DesktopStatus())
    val desktopStatus: StateFlow<DesktopStatus> = _desktopStatus.asStateFlow()

    private val _stoppedPath = MutableStateFlow<String?>(null)
    val stoppedPath: StateFlow<String?> = _stoppedPath.asStateFlow()

    /** Dernière erreur renvoyée par le bureau (commande refusée…). */
    private val _lastError = MutableStateFlow<String?>(null)
    val lastError: StateFlow<String?> = _lastError.asStateFlow()

    private var webSocket: WebSocket? = null
    /** L'utilisateur veut être connecté : on se reconnecte après une coupure. */
    private var wantConnected = false
    private var reconnectJob: Job? = null
    private var reconnectAttempt = 0

    private val httpClient = OkHttpClient.Builder()
        .connectTimeout(5, TimeUnit.SECONDS)
        .readTimeout(0, TimeUnit.SECONDS)   // connexion persistante
        .pingInterval(10, TimeUnit.SECONDS) // détecte un Wi-Fi coupé
        .build()

    fun onIpChanged(value: String) { _ip.value = value }
    fun onTokenChanged(value: String) { _token.value = value.trim().uppercase() }

    fun connectWithPairing(info: PairingInfo) {
        _ip.value = info.host
        port = info.port
        if (info.token.isNotEmpty()) _token.value = info.token
        connect()
    }

    fun connect() {
        wantConnected = true
        reconnectAttempt = 0
        openSocket()
    }

    private fun openSocket() {
        val host = _ip.value.trim()
        if (host.isEmpty()) return
        reconnectJob?.cancel()
        webSocket?.cancel()
        _connState.value = ConnectionState.Connecting
        _stoppedPath.value = null
        _lastError.value = null
        val url = "ws://$host:$port/?token=${Uri.encode(_token.value.trim())}"
        val req = Request.Builder().url(url).build()
        webSocket = httpClient.newWebSocket(req, Listener())
    }

    fun disconnect() {
        wantConnected = false
        reconnectJob?.cancel()
        webSocket?.close(1000, null)
        webSocket = null
        _connState.value = ConnectionState.Disconnected
        _desktopStatus.value = DesktopStatus()
    }

    fun send(cmd: String) {
        _lastError.value = null
        if (webSocket?.send("""{"cmd":"$cmd"}""") != true) {
            _lastError.value = "Non connecté au bureau"
        }
    }

    fun dismissError() { _lastError.value = null }

    override fun onCleared() {
        disconnect()
        httpClient.dispatcher.executorService.shutdown()
        super.onCleared()
    }

    private fun scheduleReconnect(reason: String) {
        if (!wantConnected) return
        val delayMs = (1_000L shl reconnectAttempt.coerceAtMost(4)).coerceAtMost(15_000L)
        reconnectAttempt++
        _connState.value = ConnectionState.Failed(
            "$reason — nouvelle tentative dans ${delayMs / 1000} s", retrying = true,
        )
        reconnectJob?.cancel()
        reconnectJob = viewModelScope.launch {
            delay(delayMs)
            if (wantConnected) openSocket()
        }
    }

    // ── WebSocket listener ────────────────────────────────────────────────────

    private inner class Listener : WebSocketListener() {

        // Les callbacks d'une ancienne connexion (fermée lors d'une reconnexion)
        // ne doivent pas écraser l'état de la connexion courante.
        private fun isCurrent(ws: WebSocket) = ws === this@RemoteViewModel.webSocket

        override fun onOpen(webSocket: WebSocket, response: Response) {
            if (!isCurrent(webSocket)) return
            reconnectAttempt = 0
            _connState.value = ConnectionState.Connected
            prefs.edit()
                .putString(KEY_LAST_IP, _ip.value.trim())
                .putInt(KEY_LAST_PORT, port)
                .putString(KEY_TOKEN, _token.value.trim())
                .apply()
        }

        override fun onMessage(webSocket: WebSocket, text: String) {
            if (!isCurrent(webSocket)) return
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
                        // les erreurs de commande ne déconnectent pas : on les affiche
                        _lastError.value = json.optString("message", "Erreur")
                    }
                }
            } catch (_: Exception) { /* message malformé, ignorer */ }
        }

        override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
            webSocket.close(1000, null)
            if (!isCurrent(webSocket)) return
            _desktopStatus.value = DesktopStatus()
            if (wantConnected) scheduleReconnect("Connexion fermée par le bureau")
            else _connState.value = ConnectionState.Disconnected
        }

        override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
            if (!isCurrent(webSocket)) return
            _desktopStatus.value = DesktopStatus()
            if (response?.code == HTTP_UNAUTHORIZED) {
                // Mauvais code : inutile de réessayer en boucle.
                wantConnected = false
                _connState.value = ConnectionState.Failed("Code d'appairage invalide — rescanner le QR code")
                return
            }
            scheduleReconnect(t.message ?: "Connexion échouée")
        }
    }
}
