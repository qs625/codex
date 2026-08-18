package dev.morpheus.androidcompanion.rpc

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.TimeoutCancellationException
import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.withTimeout
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.longOrNull
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicLong

class AppServerRpcClient(
    private val endpoint: String,
    private val bearerToken: String?,
    private val clientInfo: ClientInfo = ClientInfo(
        name = "morpheus_android_companion",
        title = "Morpheus Android Companion",
        version = "0.1.0",
    ),
    private val okHttpClient: OkHttpClient = OkHttpClient(),
    private val json: Json = Json {
        ignoreUnknownKeys = true
        explicitNulls = false
    },
) : AppServerConnection {
    private val nextId = AtomicLong(1)
    private val pending = ConcurrentHashMap<Long, CompletableDeferred<JsonElement>>()
    private val notifications = MutableSharedFlow<RpcNotification>(
        replay = 0,
        extraBufferCapacity = 128,
        onBufferOverflow = BufferOverflow.DROP_OLDEST,
    )
    private val events = MutableSharedFlow<RpcConnectionEvent>(
        replay = 0,
        extraBufferCapacity = 8,
        onBufferOverflow = BufferOverflow.DROP_OLDEST,
    )
    private val opened = CompletableDeferred<Unit>()
    private var webSocket: WebSocket? = null

    override val serverNotifications: SharedFlow<RpcNotification> = notifications.asSharedFlow()
    override val connectionEvents: SharedFlow<RpcConnectionEvent> = events.asSharedFlow()

    suspend fun connect() {
        connect(timeoutMs = 10_000)
    }

    override suspend fun connect(timeoutMs: Long) {
        val requestBuilder = Request.Builder().url(endpoint)
        bearerToken?.trim()?.takeIf { it.isNotEmpty() }?.let {
            requestBuilder.header("Authorization", "Bearer $it")
        }
        webSocket = okHttpClient.newWebSocket(requestBuilder.build(), Listener())
        try {
            withTimeout(timeoutMs) { opened.await() }
            request(
                method = "initialize",
                params = buildJsonObject {
                    put(
                        "clientInfo",
                        buildJsonObject {
                            put("name", JsonPrimitive(clientInfo.name))
                            put("title", JsonPrimitive(clientInfo.title))
                            put("version", JsonPrimitive(clientInfo.version))
                        },
                    )
                    put(
                        "capabilities",
                        buildJsonObject {
                            put("experimentalApi", JsonPrimitive(true))
                        },
                    )
                },
                timeoutMs = timeoutMs,
            )
            notify("initialized")
        } catch (error: TimeoutCancellationException) {
            close()
            throw RpcConnectionException("Timed out connecting to app-server", error)
        }
    }

    suspend fun request(method: String): JsonElement {
        return request(method, JsonObject(emptyMap()), 30_000)
    }

    suspend fun request(method: String, params: JsonElement): JsonElement {
        return request(method, params, 30_000)
    }

    override suspend fun request(
        method: String,
        params: JsonElement,
        timeoutMs: Long,
    ): JsonElement {
        val socket = webSocket ?: throw RpcConnectionException("WebSocket is not connected")
        val id = nextId.getAndIncrement()
        val deferred = CompletableDeferred<JsonElement>()
        pending[id] = deferred
        val payload = buildJsonObject {
            put("id", JsonPrimitive(id))
            put("method", JsonPrimitive(method))
            put("params", params)
        }
        if (!socket.send(json.encodeToString(JsonElement.serializer(), payload))) {
            pending.remove(id)
            throw RpcConnectionException("Failed to send $method request")
        }
        return try {
            withTimeout(timeoutMs) { deferred.await() }
        } finally {
            pending.remove(id)
        }
    }

    fun notify(method: String) {
        notify(method, null)
    }

    override fun notify(method: String, params: JsonElement?) {
        val socket = webSocket ?: throw RpcConnectionException("WebSocket is not connected")
        val payload = buildJsonObject {
            put("method", JsonPrimitive(method))
            if (params != null) {
                put("params", params)
            }
        }
        if (!socket.send(json.encodeToString(JsonElement.serializer(), payload))) {
            throw RpcConnectionException("Failed to send $method notification")
        }
    }

    override fun close() {
        webSocket?.close(1000, "client closed")
        webSocket = null
        failPending(RpcConnectionException("WebSocket closed"))
        okHttpClient.dispatcher.executorService.shutdown()
    }

    private fun handleMessage(text: String) {
        val root = runCatching { json.parseToJsonElement(text).jsonObject }.getOrNull() ?: return
        val id = root["id"]?.jsonPrimitive?.longOrNull
        if (id != null) {
            val deferred = pending.remove(id) ?: return
            val error = root["error"]?.jsonObject
            if (error != null) {
                deferred.completeExceptionally(
                    RpcError(
                        code = error["code"]?.jsonPrimitive?.longOrNull ?: -1,
                        message = error["message"]?.jsonPrimitive?.content ?: "JSON-RPC error",
                        data = error["data"],
                    ),
                )
            } else {
                deferred.complete(root["result"] ?: JsonNull)
            }
            return
        }

        val method = root["method"]?.jsonPrimitive?.content ?: return
        notifications.tryEmit(RpcNotification(method, root["params"]))
    }

    private fun failPending(error: Throwable) {
        pending.values.forEach { it.completeExceptionally(error) }
        pending.clear()
    }

    private inner class Listener : WebSocketListener() {
        override fun onOpen(webSocket: WebSocket, response: Response) {
            opened.complete(Unit)
        }

        override fun onMessage(webSocket: WebSocket, text: String) {
            handleMessage(text)
        }

        override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
            failPending(RpcConnectionException("WebSocket closed: $reason"))
            events.tryEmit(RpcConnectionEvent.Closed(code, reason))
        }

        override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
            opened.completeExceptionally(t)
            val error = RpcConnectionException("WebSocket failed", t)
            failPending(error)
            events.tryEmit(RpcConnectionEvent.Failed(error))
        }
    }
}

interface AppServerConnection : AutoCloseable {
    val serverNotifications: SharedFlow<RpcNotification>
    val connectionEvents: SharedFlow<RpcConnectionEvent>

    suspend fun connect(timeoutMs: Long = 10_000)

    suspend fun request(
        method: String,
        params: JsonElement = JsonObject(emptyMap()),
        timeoutMs: Long = 30_000,
    ): JsonElement

    fun notify(method: String, params: JsonElement? = null)
}

sealed interface RpcConnectionEvent {
    data class Closed(val code: Int, val reason: String) : RpcConnectionEvent
    data class Failed(val error: Throwable) : RpcConnectionEvent
}

data class ClientInfo(
    val name: String,
    val title: String,
    val version: String,
)
