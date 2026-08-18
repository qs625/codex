package dev.morpheus.androidcompanion.state

import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.viewModelScope
import dev.morpheus.androidcompanion.model.toConversationThread
import dev.morpheus.androidcompanion.model.toThreadSummary
import dev.morpheus.androidcompanion.rpc.AppServerConnection
import dev.morpheus.androidcompanion.rpc.AppServerRpcClient
import dev.morpheus.androidcompanion.rpc.RpcError
import dev.morpheus.androidcompanion.rpc.RpcConnectionEvent
import kotlinx.coroutines.Job
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject

typealias AppServerConnectionFactory = (endpoint: String, token: String?) -> AppServerConnection

class CompanionViewModel(
    private val settingsStore: ConnectionSettingsStore = EmptyConnectionSettingsStore,
    private val clientFactory: AppServerConnectionFactory = { endpoint, token ->
        AppServerRpcClient(endpoint, token)
    },
) : ViewModel() {
    private val savedSettings = settingsStore.load()
    private val mutableState = MutableStateFlow(
        CompanionUiState(
            connectionEndpoint = savedSettings?.endpoint.orEmpty(),
            connectionToken = savedSettings?.token.orEmpty(),
        ),
    )
    val state: StateFlow<CompanionUiState> = mutableState.asStateFlow()

    private var rpcClient: AppServerConnection? = null
    private var notificationJob: Job? = null
    private var connectionEventJob: Job? = null
    private var reconnectJob: Job? = null
    private var allowReconnect = false
    private var connectionGeneration = 0
    private var reconnectAttempt = 0
    private var lastSuccessfulSettings = savedSettings

    fun connect(endpoint: String, token: String?) {
        val normalizedEndpoint = endpoint.trim()
        val endpointError = validateConnectionEndpoint(normalizedEndpoint)
        if (endpointError != null) {
            mutableState.update { it.copy(error = endpointError) }
            return
        }
        val normalizedToken = token?.trim()?.takeIf { it.isNotEmpty() }
        connectInternal(normalizedEndpoint, normalizedToken, isReconnect = false, attempt = 0)
    }

    fun updateConnectionEndpoint(endpoint: String) {
        mutableState.update { it.copy(connectionEndpoint = endpoint) }
    }

    fun updateConnectionToken(token: String) {
        mutableState.update { it.copy(connectionToken = token) }
    }

    fun disconnect() {
        allowReconnect = false
        reconnectJob?.cancel()
        reconnectJob = null
        reconnectAttempt = 0
        connectionGeneration += 1
        closeCurrentClient()
        mutableState.update {
            CompanionUiState(
                connectionEndpoint = it.connectionEndpoint,
                connectionToken = it.connectionToken,
            )
        }
    }

    private fun connectInternal(
        endpoint: String,
        token: String?,
        isReconnect: Boolean,
        attempt: Int,
    ) {
        if (!isReconnect) {
            allowReconnect = false
            reconnectJob?.cancel()
            reconnectJob = null
            reconnectAttempt = 0
        }
        closeCurrentClient()
        mutableState.update {
            it.copy(
                connection = if (isReconnect) {
                    ConnectionState.Reconnecting(endpoint, attempt)
                } else {
                    ConnectionState.Connecting
                },
                connectionEndpoint = endpoint,
                connectionToken = token.orEmpty(),
                error = null,
            )
        }
        val generation = ++connectionGeneration
        viewModelScope.launch {
            val client = clientFactory(endpoint, token)
            try {
                client.connect()
                if (generation != connectionGeneration) {
                    client.close()
                    return@launch
                }
                rpcClient = client
                allowReconnect = true
                reconnectAttempt = 0
                val settings = ConnectionSettings(endpoint, token)
                lastSuccessfulSettings = settings
                settingsStore.save(settings)
                launchClientCollectors(client, generation)
                mutableState.update {
                    it.copy(
                        connection = ConnectionState.Connected(endpoint),
                        connectionEndpoint = endpoint,
                        connectionToken = token.orEmpty(),
                        error = null,
                    )
                }
                refreshThreads(generation)
            } catch (error: Throwable) {
                client.close()
                if (generation != connectionGeneration) {
                    return@launch
                }
                if (isReconnect && allowReconnect) {
                    mutableState.update { it.copy(error = reconnectMessage(error)) }
                    scheduleReconnect(error)
                } else {
                    allowReconnect = false
                    mutableState.update {
                        it.copy(
                            connection = ConnectionState.Failed(error.toUserMessage()),
                            error = error.toUserMessage(),
                        )
                    }
                }
            }
        }
    }

    private fun closeCurrentClient() {
        notificationJob?.cancel()
        notificationJob = null
        connectionEventJob?.cancel()
        connectionEventJob = null
        rpcClient?.close()
        rpcClient = null
    }

    private fun launchClientCollectors(client: AppServerConnection, generation: Int) {
        notificationJob = viewModelScope.launch {
            client.serverNotifications.collect { notification ->
                val projected = withContext(Dispatchers.Default) {
                    projectNotification(notification)
                }
                if (projected != null) {
                    mutableState.update { current ->
                        val (threads, selected) = applyProjectedNotification(
                            current.threads,
                            current.selectedThread,
                            projected,
                        )
                        current.copy(threads = threads, selectedThread = selected)
                    }
                }
            }
        }
        connectionEventJob = viewModelScope.launch {
            client.connectionEvents.collect { event ->
                if (generation == connectionGeneration && allowReconnect) {
                    scheduleReconnect(event.toThrowable())
                }
            }
        }
    }

    private fun scheduleReconnect(error: Throwable) {
        val settings = lastSuccessfulSettings ?: return
        if (!allowReconnect || reconnectJob?.isActive == true) return
        connectionGeneration += 1
        closeCurrentClient()
        reconnectAttempt += 1
        val attempt = reconnectAttempt
        mutableState.update {
            it.copy(
                connection = ConnectionState.Reconnecting(settings.endpoint, attempt),
                connectionEndpoint = settings.endpoint,
                connectionToken = settings.token.orEmpty(),
                error = reconnectMessage(error),
            )
        }
        reconnectJob = viewModelScope.launch {
            delay(reconnectDelayMillis(attempt))
            connectInternal(settings.endpoint, settings.token, isReconnect = true, attempt = attempt)
        }
    }

    fun refreshThreads() {
        refreshThreads(connectionGeneration)
    }

    private fun refreshThreads(generation: Int) {
        val client = rpcClient ?: return
        mutableState.update { it.copy(isLoadingThreads = true, error = null) }
        viewModelScope.launch {
            try {
                val result = client.request(
                    method = "thread/list",
                    params = buildJsonObject {
                        put("limit", JsonPrimitive(50))
                        put("sortKey", JsonPrimitive("updated_at"))
                        put("sortDirection", JsonPrimitive("desc"))
                    },
                )
                val threads = withContext(Dispatchers.Default) {
                    reduceThreadList(result)
                }
                if (!isCurrentConnection(generation, client)) {
                    return@launch
                }
                val selectedId = mutableState.value.selectedThreadId ?: threads.firstOrNull()?.id
                mutableState.update {
                    it.copy(
                        threads = threads,
                        selectedThreadId = selectedId,
                        isLoadingThreads = false,
                    )
                }
                selectedId?.let { selectThread(it, generation) }
            } catch (error: Throwable) {
                if (!isCurrentConnection(generation, client)) {
                    return@launch
                }
                mutableState.update {
                    it.copy(isLoadingThreads = false, error = error.toUserMessage())
                }
            }
        }
    }

    fun selectThread(threadId: String) {
        selectThread(threadId, connectionGeneration)
    }

    private fun selectThread(threadId: String, generation: Int) {
        val client = rpcClient ?: return
        mutableState.update {
            it.copy(selectedThreadId = threadId, isReadingThread = true, error = null)
        }
        viewModelScope.launch {
            try {
                val readResult = client.request(
                    method = "thread/read",
                    params = buildJsonObject {
                        put("threadId", JsonPrimitive(threadId))
                        put("includeTurns", JsonPrimitive(true))
                    },
                )
                val readThread = withContext(Dispatchers.Default) {
                    reduceThreadRead(readResult)
                }
                if (!isCurrentConnection(generation, client)) {
                    return@launch
                }
                mutableState.update { applySelectedThreadRead(it, threadId, readThread) }
                val resumeResult = client.request(
                    method = "thread/resume",
                    params = buildJsonObject {
                        put("threadId", JsonPrimitive(threadId))
                        put("excludeTurns", JsonPrimitive(true))
                    },
                )
                val resumedSummary = withContext(Dispatchers.Default) {
                    resumeResult
                        .let { it as? JsonObject }
                        ?.get("thread")
                        ?.let { it as? JsonObject }
                        ?.toThreadSummary()
                }
                if (!isCurrentConnection(generation, client)) {
                    return@launch
                }
                if (resumedSummary != null) {
                    mutableState.update { current ->
                        if (current.selectedThreadId == threadId) {
                            current.copy(threads = upsert(current.threads, resumedSummary))
                        } else {
                            current
                        }
                    }
                }
            } catch (error: Throwable) {
                if (!isCurrentConnection(generation, client)) {
                    return@launch
                }
                mutableState.update {
                    if (it.selectedThreadId == threadId) {
                        it.copy(isReadingThread = false, error = error.toUserMessage())
                    } else {
                        it
                    }
                }
            }
        }
    }

    private fun isCurrentConnection(generation: Int, client: AppServerConnection): Boolean {
        return generation == connectionGeneration && rpcClient === client
    }

    fun startThread(cwd: String?) {
        val client = rpcClient ?: return
        mutableState.update { it.copy(error = null) }
        viewModelScope.launch {
            try {
                val params = buildJsonObject {
                    cwd?.trim()?.takeIf { it.isNotEmpty() }?.let {
                        put("cwd", JsonPrimitive(it))
                    }
                    put("threadSource", JsonPrimitive("user"))
                }
                val result = client.request("thread/start", params)
                val thread = withContext(Dispatchers.Default) {
                    (result as? JsonObject)
                        ?.get("thread")
                        ?.let { it as? JsonObject }
                        ?.toConversationThread()
                }
                if (thread != null) {
                    mutableState.update {
                        it.copy(
                            threads = upsert(it.threads, thread.summary),
                            selectedThreadId = thread.id,
                            selectedThread = thread,
                        )
                    }
                }
            } catch (error: Throwable) {
                mutableState.update { it.copy(error = error.toUserMessage()) }
            }
        }
    }

    fun sendMessage(text: String) {
        val client = rpcClient ?: return
        val threadId = mutableState.value.selectedThreadId ?: return
        val trimmed = text.trim()
        if (trimmed.isEmpty()) return
        mutableState.update { it.copy(isSending = true, error = null) }
        viewModelScope.launch {
            try {
                client.request(
                    method = "turn/start",
                    params = buildJsonObject {
                        put("threadId", JsonPrimitive(threadId))
                        put(
                            "input",
                            buildJsonArray {
                                add(
                                    buildJsonObject {
                                        put("type", JsonPrimitive("text"))
                                        put("text", JsonPrimitive(trimmed))
                                    },
                                )
                            },
                        )
                    },
                )
                mutableState.update { it.copy(isSending = false) }
            } catch (error: Throwable) {
                mutableState.update {
                    it.copy(isSending = false, error = error.toUserMessage())
                }
            }
        }
    }

    override fun onCleared() {
        disconnect()
    }

    class Factory(
        private val settingsStore: ConnectionSettingsStore,
    ) : ViewModelProvider.Factory {
        @Suppress("UNCHECKED_CAST")
        override fun <T : ViewModel> create(modelClass: Class<T>): T {
            if (modelClass.isAssignableFrom(CompanionViewModel::class.java)) {
                return CompanionViewModel(settingsStore) as T
            }
            throw IllegalArgumentException("Unknown ViewModel class")
        }
    }
}

private fun upsert(current: List<dev.morpheus.androidcompanion.model.ThreadSummary>, next: dev.morpheus.androidcompanion.model.ThreadSummary): List<dev.morpheus.androidcompanion.model.ThreadSummary> {
    return listOf(next) + current.filterNot { it.id == next.id }
}

private fun Throwable.toUserMessage(): String {
    return when (this) {
        is RpcError -> "Server error $code: $message"
        else -> message ?: "Unexpected error"
    }
}

private fun reconnectMessage(error: Throwable): String {
    return "Connection lost. Reconnecting: ${error.toUserMessage()}"
}

private fun RpcConnectionEvent.toThrowable(): Throwable {
    return when (this) {
        is RpcConnectionEvent.Closed -> dev.morpheus.androidcompanion.rpc.RpcConnectionException("WebSocket closed")
        is RpcConnectionEvent.Failed -> error
    }
}

private fun reconnectDelayMillis(attempt: Int): Long {
    val clamped = attempt.coerceIn(1, 6)
    return 1_000L shl (clamped - 1)
}
